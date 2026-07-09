//! Module loader for multi-file LOGOS projects.
//!
//! Handles resolution and loading of module sources from various URI schemes,
//! with caching to prevent duplicate loading.
//!
//! # Supported URI Schemes
//!
//! | Scheme | Example | Description |
//! |--------|---------|-------------|
//! | `file:` | `file:./geometry.md` | Local filesystem (relative) |
//! | `logos:` | `logos:std` | Built-in standard library |
//! | (none) | `geometry.md` | Defaults to `file:` scheme |
//!
//! # Security
//!
//! The loader prevents path traversal attacks by checking that resolved
//! paths remain within the project root directory.
//!
//! # Caching
//!
//! Modules are cached by their normalized URI. The same module loaded from
//! different base paths will be cached separately.
//!
//! # Example
//!
//! ```no_run
//! # use logicaffeine_compile::loader::Loader;
//! # use std::path::{Path, PathBuf};
//! # fn main() -> Result<(), String> {
//! # let project_root = PathBuf::from(".");
//! let mut loader = Loader::new(project_root);
//! let source = loader.resolve(Path::new("main.md"), "file:./lib/math.md")?;
//! println!("Loaded: {}", source.path.display());
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A loaded module's source content and metadata.
#[derive(Debug, Clone)]
pub struct ModuleSource {
    /// The source content of the module
    pub content: String,
    /// The resolved path (for error reporting and relative resolution)
    pub path: PathBuf,
}

/// Module loader that handles multiple URI schemes.
///
/// Caches loaded modules to prevent duplicate loading and supports
/// cycle detection through the cache.
pub struct Loader {
    /// Cache of loaded modules (URI -> ModuleSource)
    cache: HashMap<String, ModuleSource>,
    /// Root directory of the project (for relative path resolution)
    root_path: PathBuf,
}

impl Loader {
    /// Creates a new Loader with the given root path.
    pub fn new(root_path: PathBuf) -> Self {
        Loader {
            cache: HashMap::new(),
            root_path,
        }
    }

    /// Resolves a URI to a module source.
    ///
    /// Supports:
    /// - `file:./path.md` - Local filesystem (relative to base_path)
    /// - `logos:std` - Built-in standard library
    /// - `logos:core` - Built-in core types
    pub fn resolve(&mut self, base_path: &Path, uri: &str) -> Result<&ModuleSource, String> {
        // Normalize the URI for caching
        let cache_key = self.normalize_uri(base_path, uri)?;

        // Check cache first
        if self.cache.contains_key(&cache_key) {
            return Ok(&self.cache[&cache_key]);
        }

        // Load based on scheme
        let source = if uri.starts_with("file:") {
            self.load_file(base_path, uri)?
        } else if uri.starts_with("logos:") {
            self.load_intrinsic(uri)?
        } else if uri.starts_with("https://") || uri.starts_with("http://") {
            // Remote loading not supported in base loader
            return Err(format!(
                "Remote module loading not supported for '{}'. \
                 Use the CLI's 'logos fetch' command to download dependencies locally.",
                uri
            ));
        } else {
            // Default to file: scheme if no scheme provided
            self.load_file(base_path, &format!("file:{}", uri))?
        };

        // Cache and return
        self.cache.insert(cache_key.clone(), source);
        Ok(&self.cache[&cache_key])
    }

    /// Normalizes a URI for consistent caching.
    fn normalize_uri(&self, base_path: &Path, uri: &str) -> Result<String, String> {
        if uri.starts_with("file:") {
            let path_str = uri.trim_start_matches("file:");
            let base_dir = base_path.parent().unwrap_or(&self.root_path);
            let resolved = base_dir.join(path_str);
            Ok(format!("file:{}", resolved.display()))
        } else {
            Ok(uri.to_string())
        }
    }

    /// Loads a module from the local filesystem.
    fn load_file(&self, base_path: &Path, uri: &str) -> Result<ModuleSource, String> {
        let path_str = uri.trim_start_matches("file:");

        // Resolve relative to the base file's directory
        let base_dir = base_path.parent().unwrap_or(&self.root_path);
        let resolved_path = base_dir.join(path_str);

        // Security: Check that we're not escaping the root path
        let canonical_root = self.root_path.canonicalize()
            .unwrap_or_else(|_| self.root_path.clone());

        // Read the file
        let content = fs::read_to_string(&resolved_path)
            .map_err(|e| format!("Failed to read '{}': {}", resolved_path.display(), e))?;

        // Check if escaping root (after we know the file exists)
        if let Ok(canonical_path) = resolved_path.canonicalize() {
            if !canonical_path.starts_with(&canonical_root) {
                return Err(format!(
                    "Security: Cannot load '{}' - path escapes project root",
                    uri
                ));
            }
        }

        Ok(ModuleSource {
            content,
            path: resolved_path,
        })
    }

    /// Loads a built-in module (embedded at compile time).
    fn load_intrinsic(&self, uri: &str) -> Result<ModuleSource, String> {
        let name = uri.trim_start_matches("logos:");

        match name {
            "std" => Ok(ModuleSource {
                content: include_str!("../assets/std/std.md").to_string(),
                path: PathBuf::from("logos:std"),
            }),
            "core" => Ok(ModuleSource {
                content: include_str!("../assets/std/core.md").to_string(),
                path: PathBuf::from("logos:core"),
            }),
            _ => Err(format!("Unknown intrinsic module: '{}'", uri)),
        }
    }

    /// Checks if a module has already been loaded (for cycle detection).
    pub fn is_loaded(&self, uri: &str) -> bool {
        self.cache.contains_key(uri)
    }

    /// Returns all loaded module URIs (for debugging).
    pub fn loaded_modules(&self) -> Vec<&str> {
        self.cache.keys().map(|s| s.as_str()).collect()
    }
}

// ─── Standard-library prelude (Phase 10) ────────────────────────────────────
//
// The concurrency / net / io / crdt vocabulary, embedded at compile time and made
// available WITHOUT an explicit import. To keep non-stdlib programs byte-identical
// (the AOT hot-path contract), a module is prepended ONLY when the program
// references that module's vocabulary — and per-module, so a program that names a
// pure net/io type is not forced async by the concurrency helpers it never uses.
// A `## NoPrelude` line opts out entirely.

const STD_CONCURRENCY: &str = include_str!("../assets/std/concurrency.md");
const STD_NET: &str = include_str!("../assets/std/net.md");
const STD_IO: &str = include_str!("../assets/std/io.md");
const STD_CRDT: &str = include_str!("../assets/std/crdt.md");
const STD_ENV: &str = include_str!("../assets/std/env.lg");
const STD_FILE: &str = include_str!("../assets/std/file.lg");
const STD_RANDOM: &str = include_str!("../assets/std/random.lg");
const STD_TIME: &str = include_str!("../assets/std/time.lg");
const STD_CRYPTO: &str = include_str!("../assets/std/crypto.lg");
const STD_UUID: &str = include_str!("../assets/std/uuid.lg");

/// Every stdlib module that auto-imports, in stable embedding order. The trigger
/// identifiers and collision keys are not hand-maintained — they are *derived* from
/// each module's own definitions ([`defined_names`]), so dropping a new module here
/// makes its whole vocabulary live with nothing else to update. `core`/`std` are
/// deliberately absent: they redefine builtin generics (`List`/`Map`/`Result`/…) and
/// stay explicit-import (`logos:core`) to avoid double-definition.
const PRELUDE_MODULES: &[&str] = &[
    STD_CONCURRENCY,
    STD_NET,
    STD_IO,
    STD_CRDT,
    STD_ENV,
    STD_FILE,
    STD_RANDOM,
    STD_TIME,
    STD_CRYPTO,
    STD_UUID,
];

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The names a module's CODE defines, used both as auto-import triggers and collision keys
/// ("declarer wins"): helper / native procedures (`## To [native] <name>`) and the names of
/// *type definitions* (`A <Name> has …` / `A <Name> is …` / `A <Name> of [T] …`).
///
/// Enum variant constructors (`A Debug.`, `A Some (value: T).`) are deliberately NOT taken:
/// they are common English words (`Info`, `Warning`, …) and triggering an auto-import on a
/// bare mention of one would wrongly pull a whole module into an unrelated program. A
/// program names the distinctive *type* (`Severity`) — or defines its own — so the type
/// name is the safe trigger. Field lines (`a sender, which is Int.`) are lowercase and skip.
fn defined_names(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in code.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("## To ") {
            let rest = rest.strip_prefix("native ").unwrap_or(rest);
            if let Some(name) = rest.split(|c: char| c == '(' || c.is_whitespace()).next() {
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
            continue;
        }
        let after_article = t.strip_prefix("A ").or_else(|| t.strip_prefix("An "));
        if let Some(rest) = after_article {
            if let Some(word) = rest.split(|c: char| c == '(' || c == '.' || c.is_whitespace()).next() {
                if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    // A type *header* continues with `has` / `is` / `of`; a bare `A Name.`
                    // (or `A Name (fields).`) is a variant constructor — not a trigger.
                    let tail = rest[word.len()..].trim_start();
                    let is_type_header = tail.starts_with("has")
                        || tail.starts_with("is")
                        || tail.starts_with("of");
                    if is_type_header {
                        names.push(word.to_string());
                    }
                }
            }
        }
    }
    names
}

/// The names a prelude module owns (derived from its CODE, notes stripped —
/// documentation prose must never mint a trigger name).
fn module_names(src: &str) -> Vec<String> {
    defined_names(&strip_note_blocks(module_code(src)))
}

/// Does the user `source` itself define `name`? If so, the user's definition wins and the
/// owning module is not prepended — no duplicate definition, no shadowing surprise. This is
/// also what keeps the benchmark corpus (which hand-declares `## To native args`)
/// byte-identical.
fn defines(source: &str, name: &str) -> bool {
    defined_names(source).iter().any(|n| n == name)
}

/// The CODE of a module — from its first `##` section onward, dropping the
/// markdown title + leading prose. Literate Logos only skips prose *before* the
/// first section, so when modules are concatenated only the leading prose is a
/// hazard; stripping it makes the join parse cleanly. Documentation prose stays
/// in the source files.
fn module_code(md: &str) -> &str {
    if let Some(i) = md.find("\n## ") {
        &md[i + 1..]
    } else if md.starts_with("## ") {
        md
    } else {
        ""
    }
}

/// A module's code with `## Note` documentation blocks removed — what the
/// prelude actually prepends. Notes are the IDE's per-definition doc carrier
/// (see `prelude_module_sources`); the runtime prelude stays lean, note-free,
/// and byte-identical to the pre-documentation join.
fn strip_note_blocks(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_note = false;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim();
        if in_note {
            if trimmed.starts_with("## ") && trimmed != "## Note" {
                in_note = false;
                out.push_str(line);
            }
            continue;
        }
        if trimmed == "## Note" {
            in_note = true;
            continue;
        }
        out.push_str(line);
    }
    out
}

/// The full embedded prelude — every module's CODE concatenated (what
/// [`apply_prelude`] prepends), documentation notes stripped. Identical bytes
/// on every target (`include_str!` is compile-time).
pub fn prelude() -> String {
    PRELUDE_MODULES
        .iter()
        .map(|src| strip_note_blocks(module_code(src)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The RAW embedded stdlib module sources, `## Note` documentation included —
/// the seam the LSP reads literate docs from (`teach::extract_literate_docs`).
pub fn prelude_module_sources() -> &'static [&'static str] {
    PRELUDE_MODULES
}

/// Every identifier the prelude defines (across all modules) — derived from the modules.
pub fn prelude_vocabulary() -> Vec<String> {
    PRELUDE_MODULES.iter().flat_map(|src| module_names(src)).collect()
}

/// Does `source` USE `name` — call it, launch it, or name it as a type? We require `name`
/// to appear as a whole word in a *use position* (immediately called `name(`, or preceded
/// by an invocation/type keyword like `a`/`an`/`the`/`new`/`of`/`to`/`Call` or a `:`), so a
/// bare mention in prose, a string literal, or a larger identifier never drags the module
/// in. This is what lets the auto-import stay invisible without false positives.
fn references(source: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let sb = source.as_bytes();
    let mut search_from = 0;
    while let Some(off) = source[search_from..].find(name) {
        let start = search_from + off;
        let end = start + name.len();
        search_from = start + 1;
        // Whole word: the name must not be part of a larger identifier.
        if start > 0 && is_ident_byte(sb[start - 1]) {
            continue;
        }
        if end < sb.len() && is_ident_byte(sb[end]) {
            continue;
        }
        // Call form: `name(`.
        if end < sb.len() && sb[end] == b'(' {
            return true;
        }
        // Use position: preceded by an invocation/type keyword, or a `:` (param type).
        let prefix = source[..start].trim_end();
        if prefix.ends_with(':') {
            return true;
        }
        let last_word = prefix.rsplit(|c: char| c.is_whitespace()).next().unwrap_or("");
        if matches!(last_word, "a" | "an" | "the" | "new" | "of" | "to" | "Call") {
            return true;
        }
    }
    false
}

/// Prepend the stdlib modules a program actually uses. Returns the source
/// unchanged when the program references no stdlib vocabulary or opts out with
/// `## NoPrelude` (in which case the opt-out marker is stripped so it never
/// reaches the parser). This is the auto-import seam for both the interpreter and
/// the compiler.
pub fn apply_prelude(source: &str) -> std::borrow::Cow<'_, str> {
    if let Some(stripped) = strip_no_prelude(source) {
        return std::borrow::Cow::Owned(stripped);
    }
    // The names the user source itself defines — computed once. A module is prepended only
    // when the program references one of its names AND does not define any of them. This is
    // the unified rule: it makes the auto-import demand-driven (invisible), collision-safe
    // ("declarer wins" — a user `Message`/`args` is never shadowed or double-defined), and
    // idempotent (a source already carrying a module's definitions is left untouched, so
    // the AOT hot path stays byte-identical).
    let user_defined = defined_names(source);
    let mut needed: Vec<String> = Vec::new();
    for src in PRELUDE_MODULES {
        let names = module_names(src);
        let referenced = names.iter().any(|n| references(source, n));
        let defined = names.iter().any(|n| user_defined.contains(n));
        if referenced && !defined {
            needed.push(strip_note_blocks(module_code(src)));
        }
    }
    if needed.is_empty() {
        std::borrow::Cow::Borrowed(source)
    } else {
        needed.push(source.to_string());
        std::borrow::Cow::Owned(needed.join("\n\n"))
    }
}

/// If `source` has a `## NoPrelude` opt-out line, return the source with that line
/// removed; otherwise `None`.
fn strip_no_prelude(source: &str) -> Option<String> {
    if !source.lines().any(|l| l.trim() == "## NoPrelude") {
        return None;
    }
    let kept: Vec<&str> = source.lines().filter(|l| l.trim() != "## NoPrelude").collect();
    Some(kept.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_file_scheme_resolution() {
        let temp_dir = tempdir().unwrap();
        let geo_path = temp_dir.path().join("geo.md");
        fs::write(&geo_path, "## Definition\nA Point has:\n    an x, which is Int.\n").unwrap();

        let mut loader = Loader::new(temp_dir.path().to_path_buf());
        let result = loader.resolve(&temp_dir.path().join("main.md"), "file:./geo.md");

        assert!(result.is_ok(), "Should resolve file: scheme: {:?}", result);
        assert!(result.unwrap().content.contains("Point"));
    }

    #[test]
    fn test_logos_std_scheme() {
        let mut loader = Loader::new(PathBuf::from("."));
        let result = loader.resolve(&PathBuf::from("main.md"), "logos:std");

        assert!(result.is_ok(), "Should resolve logos:std: {:?}", result);
    }

    #[test]
    fn test_logos_core_scheme() {
        let mut loader = Loader::new(PathBuf::from("."));
        let result = loader.resolve(&PathBuf::from("main.md"), "logos:core");

        assert!(result.is_ok(), "Should resolve logos:core: {:?}", result);
    }

    #[test]
    fn test_unknown_intrinsic() {
        let mut loader = Loader::new(PathBuf::from("."));
        let result = loader.resolve(&PathBuf::from("main.md"), "logos:unknown");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown intrinsic"));
    }

    #[test]
    fn test_caching() {
        let temp_dir = tempdir().unwrap();
        let geo_path = temp_dir.path().join("geo.md");
        fs::write(&geo_path, "content").unwrap();

        let mut loader = Loader::new(temp_dir.path().to_path_buf());

        // First load
        let _ = loader.resolve(&temp_dir.path().join("main.md"), "file:./geo.md");

        // Should be cached now
        assert!(loader.loaded_modules().len() == 1);
    }

    #[test]
    fn test_missing_file() {
        let temp_dir = tempdir().unwrap();
        let mut loader = Loader::new(temp_dir.path().to_path_buf());

        let result = loader.resolve(&temp_dir.path().join("main.md"), "file:./nonexistent.md");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read"));
    }

    // ─── Prelude auto-import internals ──────────────────────────────────────

    #[test]
    fn prelude_contains_no_note_blocks() {
        // `## Note` documentation lives in the module SOURCES (the IDE reads
        // it from `prelude_module_sources`); the runtime prelude must stay
        // lean and note-free, byte-identical to the pre-documentation join.
        assert!(
            !prelude().contains("## Note"),
            "prelude() must strip documentation notes before prepending"
        );
        for src in prelude_module_sources() {
            for name in defined_names(&strip_note_blocks(module_code(src))) {
                assert!(!name.is_empty());
            }
        }
    }

    #[test]
    fn note_stripping_is_byte_exact_around_headers() {
        let documented = "## Note\nDoes a thing.\n\n## To f (n: Int) -> Int:\n    Return n.\n";
        let bare = "## To f (n: Int) -> Int:\n    Return n.\n";
        assert_eq!(strip_note_blocks(documented), bare);

        let between = "## To a:\n    Show 1.\n\n## Note\nDoc.\n\n## To b:\n    Show 2.\n";
        let bare_between = "## To a:\n    Show 1.\n\n## To b:\n    Show 2.\n";
        assert_eq!(strip_note_blocks(between), bare_between);
    }

    #[test]
    fn derives_defined_names_per_module() {
        assert_eq!(module_names(STD_NET), vec!["Message"]);
        assert_eq!(module_names(STD_CRDT), vec!["Delta"]);
        // Only the distinctive type name triggers io — never its common-word variants.
        assert_eq!(module_names(STD_IO), vec!["Severity"]);
        assert_eq!(module_names(STD_CONCURRENCY), vec!["flush"]);
        assert_eq!(module_names(STD_ENV), vec!["get", "args"]);
        assert_eq!(module_names(STD_FILE), vec!["read", "write"]);
        assert_eq!(module_names(STD_RANDOM), vec!["randomInt", "randomFloat"]);
        assert_eq!(module_names(STD_TIME), vec!["now", "sleep"]);
    }

    #[test]
    fn references_matches_type_and_call_positions() {
        assert!(references("Let m be a new Message with sender 1.", "Message"));
        assert!(references("## To rank (s: Severity) -> Int:", "Severity"));
        assert!(references("Let xs be args().", "args"));
        assert!(references("Call flush with xs and ch.", "flush"));
        assert!(references("Launch a task to flush.", "flush"));
    }

    #[test]
    fn references_ignores_prose_and_substrings() {
        // A bare mention in a string is not a use position.
        assert!(!references("Show \"Message received\".", "Message"));
        // Part of a larger identifier is not a whole-word match.
        assert!(!references("Let MessageBox be 1.", "Message"));
        assert!(!references("Let nowhere be 1.", "now"));
    }

    #[test]
    fn defines_detects_native_decl_and_type() {
        assert!(defines("## To native args -> Seq of Text\n## Main\n    Show 1.", "args"));
        assert!(defines("## Definition\nA Message has:\n    a kind, which is Int.", "Message"));
        assert!(!defines("## Main\n    Let x be 1.", "Message"));
    }
}
