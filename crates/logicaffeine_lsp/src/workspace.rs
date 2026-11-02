//! Workspace-wide symbol index: every `.lg`/`.md` file under the workspace
//! folders, analyzed in the background, so symbols resolve across files the
//! user never opened.
//!
//! Open-document state stays authoritative in [`crate::state::ServerState`];
//! this index answers only what per-document resolution cannot — workspace
//! symbol queries and cross-file definition lookups.

use std::path::{Path, PathBuf};

use dashmap::DashMap;
use lsp_types::{Location, Range, SymbolKind, Url};

use crate::index::DefinitionKind;
use crate::line_index::LineIndex;
use crate::pipeline;

/// A module that publicly exports a given name — the backbone of the auto-import
/// quickfix, cross-module completion, and "defined in module X" hover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleExport {
    /// The file that defines the name.
    pub module_url: Url,
    /// The module's import alias (`util.lg` → `Util`).
    pub module_name: String,
}

/// Read-only view of the workspace's imports/exports, so LSP providers (auto-import,
/// completion, hover) are unit-testable against a fake without a running server.
pub trait WorkspaceView {
    /// Every module that publicly exports `name` (a function/type/variant).
    fn module_exporting(&self, name: &str) -> Vec<ModuleExport>;
    /// The module aliases the file at `uri` imports.
    fn imports_of(&self, uri: &Url) -> Vec<String>;
    /// Every indexed module: its import alias and file Url.
    fn module_names(&self) -> Vec<(String, Url)>;
    /// The public names the module at `uri` exports. Default empty (fakes that
    /// don't need it are unaffected).
    fn exports_of(&self, _uri: &Url) -> Vec<String> {
        Vec::new()
    }
    /// The symbol labels to offer for this file's imports (label, module alias).
    /// Default: whole-module semantics (every export of every imported module) —
    /// a fake that only implements `imports_of`/`exports_of` keeps its behavior.
    /// The real index overrides this to honor selective imports.
    fn imported_symbols(&self, uri: &Url) -> Vec<(String, String)> {
        let module_urls: std::collections::HashMap<String, Url> =
            self.module_names().into_iter().collect();
        let mut out = Vec::new();
        for alias in self.imports_of(uri) {
            if let Some(url) = module_urls.get(&alias) {
                for name in self.exports_of(url) {
                    out.push((name, alias.clone()));
                }
            }
        }
        out
    }

    /// Whether the file at `uri` imports a module that publicly exports `name`.
    fn is_imported(&self, uri: &Url, name: &str) -> bool {
        let importers = self.imports_of(uri);
        self.module_exporting(name)
            .iter()
            .any(|e| importers.iter().any(|a| a == &e.module_name))
    }
}

/// The filesystem path a `file://` URL names. `None` for any other URL, and always on a
/// target with no filesystem — the browser Studio, which has no workspace on disk.
pub fn url_path(url: &Url) -> Option<PathBuf> {
    #[cfg(any(unix, windows, target_os = "redox", target_os = "wasi"))]
    return url.to_file_path().ok();
    #[cfg(not(any(unix, windows, target_os = "redox", target_os = "wasi")))]
    {
        let _ = url;
        None
    }
}

/// The `file://` URL for a filesystem path — `None` where there is no filesystem.
pub fn path_url(path: &Path) -> Option<Url> {
    #[cfg(any(unix, windows, target_os = "redox", target_os = "wasi"))]
    return Url::from_file_path(path).ok();
    #[cfg(not(any(unix, windows, target_os = "redox", target_os = "wasi")))]
    {
        let _ = path;
        None
    }
}

/// Derive a module's import alias from its file path (`geometry.lg` → `Geometry`).
pub fn module_alias_from_url(url: &Url) -> Option<String> {
    let path = url_path(url)?;
    let stem = path.file_stem()?.to_str()?;
    let mut chars = stem.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().collect::<String>() + chars.as_str())
}

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
    /// Per-file import declarations (from the abstract's `Import` statements) —
    /// the FULL dependency, so selective imports (`Import double from Util.`) keep
    /// their selected-name list for scope-accurate completion.
    imports: DashMap<Url, Vec<logicaffeine_language::analysis::Dependency>>,
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
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
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
                if entry
                    .metadata()
                    .map(|m| m.len() > MAX_FILE_BYTES)
                    .unwrap_or(true)
                {
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
        let Some(uri) = path_url(path) else { return };
        let Ok(source) = std::fs::read_to_string(path) else {
            self.files.remove(&uri);
            return;
        };
        let analysis = pipeline::analyze(&source);
        let line_index = LineIndex::new(&source);

        // Private definitions (`## To private …` / `## A[n] private …`) resolve
        // inside their own file but are NOT importable — so they must never enter
        // the workspace surface that feeds cross-module completion and auto-import.
        let private_names: std::collections::HashSet<String> =
            logicaffeine_compile::loader::defined_names_with_visibility(&source)
                .into_iter()
                .filter(|(_, private)| *private)
                .map(|(name, _)| name)
                .collect();

        let symbols = analysis
            .symbol_index
            .definitions
            .iter()
            .filter(|def| def.span != logicaffeine_language::token::Span::default())
            .filter(|def| !private_names.contains(&def.name))
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
                    // A component is the unit a page is built from — the workspace-wide search
                    // for "Card" should find the card, exactly as it finds a struct.
                    DefinitionKind::Component => SymbolKind::CLASS,
                    DefinitionKind::Action => SymbolKind::METHOD,
                    DefinitionKind::StateField => SymbolKind::PROPERTY,
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

        let imports = logicaffeine_language::analysis::scan_dependencies(&source);

        self.files.insert(uri.clone(), symbols);
        self.refs.insert(uri.clone(), references);
        self.imports.insert(uri, imports);
    }

    pub fn remove(&self, uri: &Url) {
        self.files.remove(uri);
        self.refs.remove(uri);
        self.imports.remove(uri);
    }

    /// Every module that publicly exports `name` (a function/type/variant).
    pub fn module_exporting(&self, name: &str) -> Vec<ModuleExport> {
        let mut out = Vec::new();
        for entry in self.files.iter() {
            let exports_it = entry.value().iter().any(|s| {
                s.name == name
                    && matches!(
                        s.kind,
                        SymbolKind::FUNCTION
                            | SymbolKind::STRUCT
                            | SymbolKind::ENUM
                            | SymbolKind::ENUM_MEMBER
                    )
            });
            if exports_it {
                if let Some(alias) = module_alias_from_url(entry.key()) {
                    out.push(ModuleExport {
                        module_url: entry.key().clone(),
                        module_name: alias,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.module_name.cmp(&b.module_name));
        out
    }

    /// The module aliases the file at `uri` imports.
    pub fn imports_of(&self, uri: &Url) -> Vec<String> {
        self.imports
            .get(uri)
            .map(|v| v.iter().map(|d| d.alias.clone()).collect())
            .unwrap_or_default()
    }

    /// The exact symbol labels to complete for this file's imports, paired with
    /// the module they come from. A SELECTIVE `Import double from Util.` offers
    /// only `double` (or its `as` alias); a whole-module `Import Util.` offers
    /// every public export of Util — so completion never suggests a name the
    /// import didn't actually bring into scope.
    pub fn imported_symbols(&self, uri: &Url) -> Vec<(String, String)> {
        let module_urls: std::collections::HashMap<String, Url> =
            self.module_names().into_iter().collect();
        let mut out = Vec::new();
        let Some(deps) = self.imports.get(uri) else {
            return out;
        };
        for dep in deps.iter() {
            match &dep.selection {
                Some(selection) => {
                    for s in selection {
                        let label = s.alias.clone().unwrap_or_else(|| s.name.clone());
                        out.push((label, dep.alias.clone()));
                    }
                }
                None => {
                    if let Some(url) = module_urls.get(&dep.alias) {
                        for name in self.exports_of(url) {
                            out.push((name, dep.alias.clone()));
                        }
                    }
                }
            }
        }
        out
    }

    /// The public names the module at `uri` exports (function/type/variant).
    pub fn exports_of(&self, uri: &Url) -> Vec<String> {
        self.files
            .get(uri)
            .map(|symbols| {
                symbols
                    .iter()
                    .filter(|s| {
                        matches!(
                            s.kind,
                            SymbolKind::FUNCTION
                                | SymbolKind::STRUCT
                                | SymbolKind::ENUM
                                | SymbolKind::ENUM_MEMBER
                        )
                    })
                    .map(|s| s.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every indexed module: its import alias and file Url.
    pub fn module_names(&self) -> Vec<(String, Url)> {
        let mut out: Vec<(String, Url)> = self
            .files
            .iter()
            .filter_map(|e| module_alias_from_url(e.key()).map(|a| (a, e.key().clone())))
            .collect();
        out.sort();
        out.dedup();
        out
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
                    locations.push(Location {
                        uri: uri.clone(),
                        range: reference.range,
                    });
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

impl WorkspaceView for WorkspaceIndex {
    fn module_exporting(&self, name: &str) -> Vec<ModuleExport> {
        WorkspaceIndex::module_exporting(self, name)
    }
    fn imports_of(&self, uri: &Url) -> Vec<String> {
        WorkspaceIndex::imports_of(self, uri)
    }
    fn module_names(&self) -> Vec<(String, Url)> {
        WorkspaceIndex::module_names(self)
    }
    fn exports_of(&self, uri: &Url) -> Vec<String> {
        WorkspaceIndex::exports_of(self, uri)
    }
    fn imported_symbols(&self, uri: &Url) -> Vec<(String, String)> {
        WorkspaceIndex::imported_symbols(self, uri)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/workspace/tests.rs"]
mod tests;
