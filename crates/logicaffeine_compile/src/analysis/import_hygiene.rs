//! Import-hygiene lints — the pit-of-success guardrails that keep a project's
//! import blocks honest. An UNRESOLVABLE import is already a hard error at
//! assembly (with a "did you mean" suggestion from the loader); this pass finds
//! the problems that ASSEMBLE FINE but are still mistakes: an import that is never
//! used, the same module imported twice, and a module importing itself.
//!
//! The pass is deliberately separate from [`assemble_project`](super::assemble)
//! so the assembler stays a pure Result-or-Err function; `largo check` runs this
//! on top and surfaces the findings (warnings keep exit 0; errors fail).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::analysis::scan_dependencies;
use crate::loader::{defined_names, module_code, strip_note_blocks, DepRoot, Loader};

/// The severity of an import lint. Warnings never block a build; errors do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintSeverity {
    Warning,
    Error,
}

/// What kind of import-hygiene problem was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportLintKind {
    /// An import whose names are never referenced in the file.
    Unused,
    /// The same module imported more than once.
    Duplicate,
    /// A module that imports itself.
    SelfImport,
}

/// One import-hygiene finding.
#[derive(Debug, Clone)]
pub struct ImportLint {
    pub kind: ImportLintKind,
    pub severity: LintSeverity,
    /// The module alias the import binds (`Util`).
    pub module: String,
    /// The human-facing message.
    pub message: String,
    /// The `[start, end)` byte span of the import statement (for a fmt/LSP fix).
    pub span: (usize, usize),
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `name` appears as a whole word anywhere in `code`.
fn word_appears(code: &str, name: &str) -> bool {
    code.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| w == name)
}

/// Lint the entry file's imports for unused / duplicate / self-import problems.
/// Unresolvable imports are NOT reported here — the assembler already rejects
/// those with a suggestion. Returns `Err` only if the entry can't be read.
pub fn lint_imports(entry: &Path) -> Result<Vec<ImportLint>, String> {
    lint_imports_with_deps(entry, Vec::new())
}

/// [`lint_imports`], resolving imports with the declared dependency roots so a
/// `path`/`git`-local import is lint-checked too. `dep_roots` empty is identical
/// to [`lint_imports`].
pub fn lint_imports_with_deps(
    entry: &Path,
    dep_roots: Vec<DepRoot>,
) -> Result<Vec<ImportLint>, String> {
    let raw = std::fs::read_to_string(entry)
        .map_err(|e| format!("could not read `{}`: {e}", entry.display()))?;
    let root = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader::with_dep_roots(root, dep_roots);
    let entry_canon = canonical(entry);

    // The entry's own code — where a reference to an imported name would appear.
    let code = strip_note_blocks(module_code(&raw));

    let mut lints = Vec::new();
    let mut seen: HashMap<PathBuf, String> = HashMap::new();

    for dep in scan_dependencies(&raw) {
        let span = (dep.start, dep.end);
        let (resolved, content) = match loader.resolve(entry, &dep.uri) {
            Ok(m) => (canonical(&m.path), m.content.clone()),
            // Unresolvable — the assembler will report it (with a suggestion).
            Err(_) => continue,
        };

        if resolved == entry_canon {
            lints.push(ImportLint {
                kind: ImportLintKind::SelfImport,
                severity: LintSeverity::Error,
                module: dep.alias.clone(),
                message: format!("`{}` imports itself — remove the self-import.", dep.alias),
                span,
            });
            continue;
        }

        if let Some(first) = seen.get(&resolved) {
            lints.push(ImportLint {
                kind: ImportLintKind::Duplicate,
                severity: LintSeverity::Warning,
                module: dep.alias.clone(),
                message: format!(
                    "`{}` is imported more than once (already imported as `{first}`) — remove the duplicate.",
                    dep.alias
                ),
                span,
            });
            continue;
        }
        seen.insert(resolved, dep.alias.clone());

        // Which names this import brings — the selected ones (surface alias) for a
        // selective import, else every public name the module exports.
        let names: Vec<String> = match &dep.selection {
            Some(sel) => sel
                .iter()
                .map(|s| s.alias.clone().unwrap_or_else(|| s.name.clone()))
                .collect(),
            None => defined_names(&strip_note_blocks(module_code(&content))),
        };
        if !names.is_empty() && !names.iter().any(|n| word_appears(&code, n)) {
            lints.push(ImportLint {
                kind: ImportLintKind::Unused,
                severity: LintSeverity::Warning,
                module: dep.alias.clone(),
                message: format!(
                    "`Import {}` is unused — no name from it is referenced here.",
                    dep.alias
                ),
                span,
            });
        }
    }

    Ok(lints)
}
