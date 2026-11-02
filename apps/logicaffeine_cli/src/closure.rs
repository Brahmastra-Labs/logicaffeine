//! ONE assembled module closure for every `largo` command (G5).
//!
//! [`assemble_closure`] is the single seam test / serve / check / emit / build
//! consume: the entry's whole import closure assembled into one compilation
//! unit (via [`logicaffeine_compile::analysis::assemble_project`]) PLUS the
//! on-disk file set behind it, so a watcher can track every file whose edit
//! changes the program — not just the entry or its immediate directory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use logicaffeine_compile::analysis::dependencies::scan_dependencies;
use logicaffeine_compile::loader::{DepRoot, Loader};

/// One assembled module closure: the combined source string every tier
/// compiles, and every on-disk file that contributed to it.
#[derive(Debug)]
pub(crate) struct Closure {
    /// The single combined source (entry + every transitively imported
    /// module, dependency-first) — the same string `largo build`/`run` feed
    /// the compile pipeline.
    pub source: String,
    /// Every filesystem file in the closure: the entry first, then each
    /// imported module exactly once (canonical paths; cycle- and
    /// diamond-safe). Intrinsic (`logos:`) and remote modules have no local
    /// file and are not listed.
    pub files: Vec<PathBuf>,
    /// Byte-range → originating file mapping over [`source`](Self::source), so a
    /// diagnostic on the combined blob (e.g. a deep-check finding) is attributed
    /// back to the user's own file and line rather than the spliced unit.
    pub source_map: logicaffeine_compile::analysis::SourceMap,
}

/// Assemble the whole project reachable from `entry` into one closure.
/// Errors only for unreadable/unresolvable imports — the same failures
/// `assemble_project` reports.
pub(crate) fn assemble_closure(entry: &Path) -> Result<Closure, String> {
    assemble_closure_with_deps(entry, Vec::new())
}

/// [`assemble_closure`], additionally linking the declared dependency roots so a
/// bare `Import <Dep>.` reaches a `path`/`git`-local package outside the project
/// root. `dep_roots` empty is byte-for-byte identical to [`assemble_closure`], so
/// a single-project build is unchanged. THIS is the single seam every tier
/// consumes — the linked closure is identical across interp / VM / check / emit /
/// build by construction.
pub(crate) fn assemble_closure_with_deps(
    entry: &Path,
    dep_roots: Vec<DepRoot>,
) -> Result<Closure, String> {
    let assembled =
        logicaffeine_compile::analysis::assemble_project_with_deps(entry, dep_roots.clone())?;
    let files = closure_files(entry, dep_roots)?;
    Ok(Closure {
        source: assembled.source,
        files,
        source_map: assembled.source_map,
    })
}

/// The on-disk files of the import closure: DFS over `Import` declarations
/// with the same loader resolution `assemble_project` uses, deduped by
/// canonical path so a diamond or cycle contributes each file once.
fn closure_files(entry: &Path, dep_roots: Vec<DepRoot>) -> Result<Vec<PathBuf>, String> {
    let root = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader::with_dep_roots(root, dep_roots);

    let entry_raw = std::fs::read_to_string(entry)
        .map_err(|e| format!("Failed to read entry file '{}': {}", entry.display(), e))?;
    let entry_canon = canonicalize(entry);

    let mut visited: HashSet<PathBuf> = HashSet::new();
    visited.insert(entry_canon.clone());
    let mut files = vec![entry_canon];

    let mut pending: Vec<(PathBuf, String)> = vec![(entry.to_path_buf(), entry_raw)];
    while let Some((base, raw)) = pending.pop() {
        for dep in scan_dependencies(&raw) {
            let (path, content) = {
                let module = loader.resolve(&base, &dep.uri)?;
                (module.path.clone(), module.content.clone())
            };
            let canon = canonicalize(&path);
            if visited.insert(canon.clone()) {
                // Only real files are watchable; an intrinsic (`logos:`) or
                // remote module carries a pseudo-path.
                if canon.exists() {
                    files.push(canon);
                }
                pending.push((path, content));
            }
        }
    }
    Ok(files)
}

/// Canonicalize for dedup, falling back to the path itself (mirrors the
/// assembler's own dedup key).
fn canonicalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_cli/closure/tests.rs"]
mod tests;
