//! Workspace-wide symbol index: every `.lg`/`.md` file under the workspace
//! folders, analyzed in the background, so symbols resolve across files the
//! user never opened.
//!
//! Open-document state stays authoritative in [`crate::state::ServerState`];
//! this index answers only what per-document resolution cannot — workspace
//! symbol queries and cross-file definition lookups.

use std::path::Path;

use dashmap::DashMap;
use tower_lsp::lsp_types::{Location, Range, SymbolKind, Url};

use crate::index::DefinitionKind;
use crate::line_index::LineIndex;
use crate::pipeline;

/// Bounds: a workspace scan must never wedge the server on a monorepo.
const MAX_FILES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const SKIP_DIRS: &[&str] = &["target", "node_modules", "dist", "out", "logs"];

pub struct WorkspaceSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub container: Option<String>,
    pub location: Location,
}

/// One name occurrence in a file, for cross-file references and rename.
pub struct FileReference {
    pub name: String,
    pub range: Range,
    /// True when the file's own scope resolved this reference — its target
    /// is local unless this very file defines the workspace-visible symbol.
    pub resolved_locally: bool,
}

#[derive(Default)]
pub struct WorkspaceIndex {
    files: DashMap<Url, Vec<WorkspaceSymbol>>,
    refs: DashMap<Url, Vec<FileReference>>,
}

impl WorkspaceIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyze every LOGOS file under `root` (bounded). Blocking — run it on
    /// a blocking task.
    pub fn scan_folder(&self, root: &Path) {
        let mut pending = vec![root.to_path_buf()];
        let mut seen_files = 0usize;
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() {
                    if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_ref()) {
                        pending.push(path);
                    }
                    continue;
                }
                let is_logos = matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("lg") | Some("md")
                );
                if !is_logos {
                    continue;
                }
                if entry.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                    continue;
                }
                if seen_files >= MAX_FILES {
                    log::warn!(
                        "workspace scan hit the {MAX_FILES}-file bound at {}; remaining files are unindexed",
                        path.display()
                    );
                    return;
                }
                seen_files += 1;
                self.index_file(&path);
            }
        }
    }

    /// (Re-)analyze one file into the index.
    pub fn index_file(&self, path: &Path) {
        let Ok(uri) = Url::from_file_path(path) else { return };
        let Ok(source) = std::fs::read_to_string(path) else {
            self.files.remove(&uri);
            return;
        };
        let analysis = pipeline::analyze(&source);
        let line_index = LineIndex::new(&source);

        let symbols = analysis
            .symbol_index
            .definitions
            .iter()
            .filter(|def| def.span != logicaffeine_language::token::Span::default())
            .filter_map(|def| {
                // Locals stay per-document; the workspace surface is API shape.
                let kind = match def.kind {
                    DefinitionKind::Function => SymbolKind::FUNCTION,
                    DefinitionKind::Struct => SymbolKind::STRUCT,
                    DefinitionKind::Enum => SymbolKind::ENUM,
                    DefinitionKind::Field => SymbolKind::FIELD,
                    DefinitionKind::Variant => SymbolKind::ENUM_MEMBER,
                    DefinitionKind::Theorem => SymbolKind::CLASS,
                    DefinitionKind::Block => SymbolKind::NAMESPACE,
                    DefinitionKind::Variable | DefinitionKind::Parameter => return None,
                };
                Some(WorkspaceSymbol {
                    name: def.name.clone(),
                    kind,
                    container: def.detail.clone(),
                    location: Location {
                        uri: uri.clone(),
                        range: Range {
                            start: line_index.position(def.span.start),
                            end: line_index.position(def.span.end),
                        },
                    },
                })
            })
            .collect();

        let references = analysis
            .symbol_index
            .references
            .iter()
            .map(|reference| FileReference {
                name: reference.name.clone(),
                range: Range {
                    start: line_index.position(reference.span.start),
                    end: line_index.position(reference.span.end),
                },
                resolved_locally: reference.definition_idx.is_some(),
            })
            .collect();

        self.files.insert(uri.clone(), symbols);
        self.refs.insert(uri, references);
    }

    pub fn remove(&self, uri: &Url) {
        self.files.remove(uri);
        self.refs.remove(uri);
    }

    /// Does this file define `name` as workspace-visible API shape?
    fn defines(&self, uri: &Url, name: &str) -> bool {
        self.files
            .get(uri)
            .map(|symbols| symbols.iter().any(|s| s.name == name))
            .unwrap_or(false)
    }

    /// Every cross-file occurrence of `name` outside `skip` (the URIs whose
    /// LIVE buffers answer for themselves). A file's occurrence counts when
    /// its own scope did NOT resolve it (so it reaches across files), or when
    /// the file itself defines the symbol (its local uses ARE the symbol).
    pub fn references_of(&self, name: &str, skip: &[&Url]) -> Vec<Location> {
        let mut locations = Vec::new();
        for entry in self.refs.iter() {
            let uri = entry.key();
            if skip.contains(&uri) {
                continue;
            }
            let file_defines = self.defines(uri, name);
            for reference in entry.value() {
                if reference.name == name && (!reference.resolved_locally || file_defines) {
                    locations.push(Location { uri: uri.clone(), range: reference.range });
                }
            }
        }
        locations
    }

    /// Case-insensitive substring query across the workspace.
    pub fn query(&self, needle: &str, limit: usize) -> Vec<WorkspaceSymbol> {
        let needle = needle.to_lowercase();
        let mut hits = Vec::new();
        for entry in self.files.iter() {
            for symbol in entry.value() {
                if needle.is_empty() || symbol.name.to_lowercase().contains(&needle) {
                    hits.push(WorkspaceSymbol {
                        name: symbol.name.clone(),
                        kind: symbol.kind,
                        container: symbol.container.clone(),
                        location: symbol.location.clone(),
                    });
                    if hits.len() >= limit {
                        return hits;
                    }
                }
            }
        }
        hits
    }

    /// The definition location for an exact name, for cross-file goto-def.
    /// Callable (function) and type-like definitions win over incidental
    /// name matches in other kinds.
    pub fn definition_of(&self, name: &str) -> Option<Location> {
        let mut fallback = None;
        for entry in self.files.iter() {
            for symbol in entry.value() {
                if symbol.name != name {
                    continue;
                }
                match symbol.kind {
                    SymbolKind::FUNCTION | SymbolKind::STRUCT | SymbolKind::ENUM => {
                        return Some(symbol.location.clone());
                    }
                    _ => fallback = Some(symbol.location.clone()),
                }
            }
        }
        fallback
    }
}
