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

pub use crate::stdlib_text::defined_names_with_visibility;
pub(crate) use crate::stdlib_text::{defined_names, function_header, module_code, strip_note_blocks};
use crate::stdlib_text::{
    builtin_backed, fn_section_name, function_section_ranges, is_ident_byte, may_name,
    references, whole_words,
};

/// Resolve a requested module path to a concrete file: the exact path if it
/// exists, otherwise `<path>.lg` then `<path>.md` — so `Import Geometry.`
/// (whose URI is the extension-less `./geometry`) finds `geometry.lg` or
/// `geometry.md`. Failing that, a DIRECTORY / barrel module: `Import Lexer.`
/// resolves `lexer/lexer.lg` (self-descriptive), then `lexer/mod.lg`, then their
/// `.md` variants — so a whole subsystem is one import. Returns the actual file
/// path, or `None` if none exists.
///
/// File resolution is tried BEFORE the directory probe, so a real `lexer.lg`
/// beside a `lexer/` directory always wins (adding a directory never silently
/// shadows an existing module file).
fn resolve_module_file(requested: &Path) -> Option<PathBuf> {
    if requested.is_file() {
        return Some(requested.to_path_buf());
    }
    for ext in ["lg", "md"] {
        let candidate = requested.with_extension(ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if requested.is_dir() {
        if let Some(dirname) = requested.file_name().and_then(|s| s.to_str()) {
            let candidates = [
                requested.join(format!("{dirname}.lg")),
                requested.join(format!("{dirname}.md")),
                requested.join("mod.lg"),
                requested.join("mod.md"),
            ];
            for candidate in candidates {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Suggest the closest existing sibling module for a mistyped bare import
/// (`Import Geometrey.` → "did you mean `Geometry`?"). Compares case-insensitively
/// (imports are Capitalized, files lowercase) over the `.lg`/`.md` stems beside
/// the requested path, within a small edit distance.
fn suggest_sibling_module(requested: &Path, name: &str) -> Option<String> {
    let dir = requested.parent()?;
    let mut stems: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let is_module = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e == "lg" || e == "md");
        if is_module {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                stems.push(stem.to_lowercase());
            }
        }
    }
    let refs: Vec<&str> = stems.iter().map(String::as_str).collect();
    let target = name.to_lowercase();
    logicaffeine_language::suggest::find_similar(&target, &refs, 3).map(|s| {
        let mut chars = s.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            None => s.to_string(),
        }
    })
}

/// A loaded module's source content and metadata.
#[derive(Debug, Clone)]
pub struct ModuleSource {
    /// The source content of the module
    pub content: String,
    /// The resolved path (for error reporting and relative resolution)
    pub path: PathBuf,
}

/// A declared dependency's resolved root: the alias an `Import <Name>.` binds to
/// (matched case-insensitively against the bare import name) and the
/// canonicalized directory its modules live under. Populated from a project's
/// `Largo.toml` `[dependencies]`; empty for a single-project build.
#[derive(Debug, Clone)]
pub struct DepRoot {
    /// The dependency name (its `[dependencies]` key / `Import` alias).
    pub name: String,
    /// The canonicalized directory the dependency's modules live under.
    pub root: PathBuf,
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
    /// Declared dependency roots ([`DepRoot`]). An `Import <Name>.` whose bare
    /// name matches a dependency resolves against that dependency's root, and the
    /// containment gate ADMITS every declared root (canonicalized) in addition to
    /// the project root — so a linked dependency lives legally OUTSIDE the project
    /// root while an undeclared `..` escape is still refused.
    dep_roots: Vec<DepRoot>,
    /// The directory holding this project's `Largo.toml`, found by walking up from
    /// [`root_path`](Self::root_path) — `None` for a `.lg` that is not in a project
    /// (a scratch file, a doc example), which then resolves exactly as it always did.
    ///
    /// ★ IT IS WHAT MAKES A BARE `Import <Name>.` MEAN "a module of THIS PROJECT".
    /// The loader used to search only the importing file's own directory, so a page in
    /// `pages/` could not reach `src/shared.lg` — the one type a page and its server
    /// agree about was unreachable from the page. It is also the containment root, so
    /// a page may legally name a module in a sibling directory of the same project.
    project_root: Option<PathBuf>,
}

/// The directory holding the nearest `Largo.toml` at or above `start`.
///
/// Discovered rather than threaded through every caller deliberately: `largo build`, `largo test`,
/// the LSP and a plain `assemble_project` must agree about what a bare name means, and three
/// callers passing it separately is three chances for them to disagree.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join("Largo.toml").is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

impl Loader {
    /// Creates a new Loader with the given root path.
    pub fn new(root_path: PathBuf) -> Self {
        Loader {
            cache: HashMap::new(),
            project_root: find_project_root(&root_path),
            root_path,
            dep_roots: Vec::new(),
        }
    }

    /// Creates a Loader that additionally admits the declared dependency roots —
    /// the seam that links `path`/`git`-local packages into the closure.
    pub fn with_dep_roots(root_path: PathBuf, dep_roots: Vec<DepRoot>) -> Self {
        Loader {
            cache: HashMap::new(),
            project_root: find_project_root(&root_path),
            root_path,
            dep_roots,
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
        let requested = base_dir.join(path_str);

        // Resolve the concrete file: the exact path, or — for a bare `Import
        // Name` whose URI carries no extension — `<name>.lg` / `<name>.md`.
        // Failing that, a bare `Import <Dep>.` may name a DECLARED dependency,
        // whose modules live outside the project (`resolve_in_dep_roots`).
        let resolved_path = resolve_module_file(&requested)
            .or_else(|| self.resolve_in_project(&requested))
            .or_else(|| self.resolve_in_dep_roots(&requested))
            .ok_or_else(|| {
            let name = requested
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("module");
            let dir = requested
                .parent()
                .and_then(|p| p.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(".");
            if requested.extension().is_some() {
                format!("could not find the module file `{}` (looked in {dir})", requested.display())
            } else {
                let hint = suggest_sibling_module(&requested, name)
                    .map(|s| format!(" — did you mean `{s}`?"))
                    .unwrap_or_default();
                format!(
                    "could not find a module named `{name}` — looked for `{name}.lg` and `{name}.md` in {dir}{hint}"
                )
            }
        })?;

        // Security: FAIL-CLOSED path containment. Canonicalize the resolved file
        // and reject BEFORE reading if it escapes EVERY allowed root — the project
        // root PLUS each declared dependency root — or if it can't be
        // canonicalized (never fall back to a permissive check). A declared
        // dependency legally lives outside the project root; an UNdeclared `..`
        // escape matches no allowed root and is refused. `logos:` intrinsics never
        // reach here.
        let canonical_path = resolved_path.canonicalize().map_err(|_| {
            format!(
                "Security: cannot verify that `{}` is within the project root",
                resolved_path.display()
            )
        })?;
        if !self.is_within_an_allowed_root(&canonical_path)? {
            return Err(format!("Security: `{uri}` escapes the project root"));
        }

        // Read the file — only after the containment gate has passed.
        let content = fs::read_to_string(&resolved_path)
            .map_err(|e| format!("Failed to read '{}': {}", resolved_path.display(), e))?;

        Ok(ModuleSource {
            content,
            path: resolved_path,
        })
    }

    /// Resolve a bare `Import <Dep>.` (URI `file:./<dep>`) against a DECLARED
    /// dependency's root when in-project resolution failed. The import's bare name
    /// (the requested path's final component, extension stripped) is matched
    /// case-insensitively against each declared dependency, and within that
    /// dependency's root the module is found the same way the loader finds any
    /// module: `<root>/<dep>.lg|.md`, the barrel `<root>/<dep>/…`, or the
    /// dependency's own `src/<dep>.lg|.md` / `src/main.lg|.md` project layout.
    /// `None` when no dependency matches or the module isn't found in its root.
    /// A bare `Import <Name>.` that named no module beside the importing file: look for it in the
    /// PROJECT's own module directories — `src/` first, then the project root.
    ///
    /// ★ THE IMPORTING FILE'S OWN DIRECTORY IS SEARCHED FIRST, by the caller, and that order is the
    /// decision: a module beside you is the one you meant. Preferring `src/` would silently change
    /// which file an existing project's import resolves to, which is behaviour nobody edited.
    ///
    /// Only a BARE name reaches here — one with an extension is an explicit path that says exactly
    /// where to look, and second-guessing it would resolve `from "old/thing.lg"` to a different file.
    fn resolve_in_project(&self, requested: &Path) -> Option<PathBuf> {
        if requested.extension().is_some() {
            return None;
        }
        let root = self.project_root.as_ref()?;
        let name = requested.file_name()?;
        let candidates = [root.join("src").join(name), root.join(name)];
        candidates.iter().find_map(|c| resolve_module_file(c))
    }

    fn resolve_in_dep_roots(&self, requested: &Path) -> Option<PathBuf> {
        if self.dep_roots.is_empty() {
            return None;
        }
        let stem = requested
            .file_name()
            .and_then(|s| s.to_str())
            .map(|n| n.rsplit_once('.').map(|(s, _)| s).unwrap_or(n))?;
        let stem_lc = stem.to_lowercase();
        let dep = self
            .dep_roots
            .iter()
            .find(|d| d.name.to_lowercase() == stem_lc)?;
        // Candidate module locations within the dependency's root, in priority
        // order: a flat module beside the root, its `src/` project layout, then
        // the dependency's conventional entry.
        let candidates = [
            dep.root.join(stem),
            dep.root.join("src").join(stem),
            dep.root.join("src").join("main"),
        ];
        for candidate in candidates {
            if let Some(found) = resolve_module_file(&candidate) {
                return Some(found);
            }
        }
        None
    }

    /// True when `canonical_path` is contained within the project root OR any
    /// declared dependency root (all canonicalized). Fail-closed: a project root
    /// that can't be canonicalized is a hard error.
    fn is_within_an_allowed_root(&self, canonical_path: &Path) -> Result<bool, String> {
        let canonical_root = self.root_path.canonicalize().map_err(|_| {
            format!(
                "Security: cannot verify the project root `{}`",
                self.root_path.display()
            )
        })?;
        if canonical_path.starts_with(&canonical_root) {
            return Ok(true);
        }
        // The PROJECT root, when the entry sits in a subdirectory of one (`pages/app.lg` has
        // `pages/` as its base). Without it a page could not name a module in `src/` even by an
        // explicit path — the widening above would find the file and the gate would refuse it.
        // An import that escapes the project ENTIRELY is still refused, which is the gate's job.
        if let Some(project) = &self.project_root {
            if let Ok(p) = project.canonicalize() {
                if canonical_path.starts_with(&p) {
                    return Ok(true);
                }
            }
        }
        for dep in &self.dep_roots {
            // A dep root is already canonicalized at resolution time; canonicalize
            // again defensively (cheap, and robust to a caller passing a raw path).
            if let Ok(dep_root) = dep.root.canonicalize() {
                if canonical_path.starts_with(&dep_root) {
                    return Ok(true);
                }
            } else if canonical_path.starts_with(&dep.root) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Loads a built-in module (embedded at compile time).
    fn load_intrinsic(&self, uri: &str) -> Result<ModuleSource, String> {
        let name = uri.trim_start_matches("logos:");

        match name {
            "std" => Ok(ModuleSource {
                content: include_str!("../assets/std/std.lg").to_string(),
                path: PathBuf::from("logos:std"),
            }),
            "core" => Ok(ModuleSource {
                content: include_str!("../assets/std/core.lg").to_string(),
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

/// ★ THE MIME TABLE LIVES IN ONE PLACE, AND THAT PLACE IS `static.lg`. ★
///
/// A content type decides whether a browser RUNS a file, renders it, or downloads it — so it is
/// not cosmetic: a stylesheet typed `text/plain` is ignored, a wasm module typed
/// `application/octet-stream` fails `instantiateStreaming`, and a page with no charset renders a
/// visitor's name as mojibake.
///
/// TWO different things ask the question. A DEPLOYED LOGOS program answers it by calling
/// `static.lg`'s `contentTypeFor`; `largo serve` and `largo build` answer it as the tool
/// assembling a site. When those were two tables they had already DRIFTED — `text/css` against
/// `text/css; charset=utf-8`, and six types the LOGOS side did not know at all, so a deployed
/// site served its own fonts as `application/octet-stream`. A file typed one way in development
/// and another in production is the single worst place for a difference to appear.
///
/// So there is ONE table and the tool READS IT, rather than two tables held in agreement by a
/// test. `static.lg` is the source because framework logic belongs in LOGOS, where every tier
/// runs it; adding a type there is the only edit an author ever makes, and largo picks it up with
/// no second place to remember.
pub fn content_type_for(path: &str) -> &'static str {
    static TABLE: std::sync::OnceLock<Vec<(String, &'static str)>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| parse_content_types(STD_STATIC));
    let lower = path.to_ascii_lowercase();
    for (suffix, ty) in table {
        if lower.ends_with(suffix) {
            return ty;
        }
    }
    // The same download-never-guess default `contentTypeFor` ends with. Guessing `text/html` for
    // an unknown file would let an uploaded file be served as a PAGE — a stored-XSS delivery path.
    "application/octet-stream"
}

/// Read `contentTypeFor`'s `If endsWith(p, ".ext") …: Return "type".` chain out of `static.lg`,
/// in source order — the same order a LOGOS reader evaluates it in, so `.woff2` is tested before
/// `.woff` exactly as the file writes it.
fn parse_content_types(source: &str) -> Vec<(String, &'static str)> {
    let body = match source.split("## To contentTypeFor").nth(1) {
        Some(rest) => rest.split("\n## ").next().unwrap_or(rest),
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("If endsWith(p, \"") {
            if let Some(suffix) = rest.split('"').next() {
                pending.push(suffix.to_ascii_lowercase());
            }
        } else if let Some(rest) = line.strip_prefix("Return \"") {
            if let Some(ty) = rest.split('"').next() {
                // The trailing bare `Return "application/octet-stream".` has no `If` in front of
                // it: it is the default, not an extension's answer, so it binds nothing.
                let ty: &'static str = Box::leak(ty.to_string().into_boxed_str());
                for suffix in pending.drain(..) {
                    out.push((suffix, ty));
                }
            }
        }
    }
    out
}

// The prelude's modules, embedded by the build script from `stdlib_text::PRELUDE_FILES`: `STD_<STEM>` for each
// and `PRELUDE_MODULES` in splice order, with the index `stdlib_text::index` derives from them once per build
// (`PRELUDE_NAMES`, `PRELUDE_NATIVES`, `PRELUDE_PULLS`, `PRELUDE_MENTIONS_TEXT_FROM_BYTES`, `PRELUDE_SPLICE`,
// `LIBRARY_SECTIONS`).
include!(concat!(env!("OUT_DIR"), "/stdlib_index.rs"));

/// Does the user `source` itself define `name`? If so, the user's definition wins at every
/// call the program writes (see [`apply_prelude_mapped`]).
fn defines(source: &str, name: &str) -> bool {
    defined_names(source).iter().any(|n| n == name)
}

/// The names a program binds LOCALLY — the program's own names, distinct from the top-level
/// `## To`/`## A` declarations [`defined_names`] reports, split by whether a CALL can reach them.
///
/// * `callables` — `Let <name> be (<params>) -> …` closures. A call `name(…)` resolves to a
///   top-level function before a local, so a closure sharing a stdlib function's name is a
///   declaration in every sense that matters: the stdlib function must be renamed out of its way.
/// * `values` — every other `Let`, loop variable (`Repeat for x in`, `For each x in`) and
///   parameter. A value and a function of the same name coexist (`words` reads the variable,
///   `words(s)` calls the function), so such a name neither suppresses nor renames anything; it
///   only stops counting as a *use* of the stdlib name unless it is actually called
///   (`item 1 of perm` is not a use of math.lg's `perm`).
///
/// Used ONLY by [`apply_prelude`]; kept out of `defined_names` so downstream consumers (the
/// assembler's `fn_index`/closure bookkeeping) never see local variables as top-level definitions.
struct LocalBindings {
    callables: Vec<String>,
    values: Vec<String>,
}

fn local_bindings(source: &str) -> LocalBindings {
    fn plain_ident(s: &str) -> Option<&str> {
        let s = s.trim();
        (!s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(s)
    }
    let mut callables = Vec::new();
    let mut values = Vec::new();
    for line in source.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Let ") {
            let rest = rest.strip_prefix("mutable ").unwrap_or(rest);
            if let Some((name, value)) = rest.split_once(" be ") {
                if let Some(name) = plain_ident(name) {
                    let value = value.trim_start();
                    if value.starts_with('(') && value.contains(") ->") {
                        callables.push(name.to_string());
                    } else {
                        values.push(name.to_string());
                    }
                }
            }
        }
        for head in ["Repeat for ", "For each "] {
            if let Some(rest) = t.strip_prefix(head) {
                if let Some((name, _)) = rest.split_once(" in ") {
                    if let Some(name) = plain_ident(name) {
                        values.push(name.to_string());
                    }
                }
            }
        }
        // Parameters: `(name: Type` in a `## To` header or a closure literal.
        if t.starts_with("## To ") || t.contains(") ->") {
            let mut from = 0;
            while let Some(off) = t[from..].find(':') {
                let colon = from + off;
                from = colon + 1;
                let before = t[..colon].trim_end();
                let word_start = before
                    .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let opener = before[..word_start].trim_end();
                if opener.ends_with('(') || opener.ends_with(',') {
                    if let Some(name) = plain_ident(&before[word_start..]) {
                        values.push(name.to_string());
                    }
                }
            }
        }
    }
    LocalBindings { callables, values }
}

/// The reserved spelling a stdlib definition takes when the program declares the same name
/// itself. The double underscore is what no LOGOS author writes and what no stdlib name contains;
/// a type or variant keeps its capital (the parser reads a capitalized word as a type name).
fn hygienic_name(name: &str) -> String {
    if name.starts_with(|c: char| c.is_ascii_uppercase()) {
        format!("Std__{name}")
    } else {
        format!("__std_{name}")
    }
}

/// Rename, inside spliced STDLIB code, every definition of and reference to a name the program
/// declares itself, so the program's definition and the library's coexist and library code keeps
/// reaching the library's own.
///
/// * A capitalized name (type or variant) is renamed at every whole-word code occurrence — field
///   and local names are lowercase, so a capitalized collision can only ever mean the type.
/// * A lowercase name (function) is renamed only where it names the FUNCTION: its `## To` header,
///   a call `name(`, `Call name`, and a spawn `task to name`. A stdlib local or field that happens
///   to share the name (`pi`, `gamma`, `cookie`) is a different entity and is left alone; the
///   stdlib never passes its own functions as values (locked by `stdlib_user_name_hygiene`).
///
/// String literals are opaque except for `{…}` interpolation holes, which are code. Char literals
/// (`` `"` ``) are opaque.
fn rename_for_hygiene(code: &str, renames: &std::collections::HashMap<String, String>) -> String {
    if renames.is_empty() {
        return code.to_string();
    }
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len() + renames.len() * 16);
    let mut copied = 0;
    let mut in_string = false;
    let mut interp_depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string && interp_depth == 0 {
            match b {
                b'\\' => i += 2,
                b'"' => {
                    in_string = false;
                    i += 1;
                }
                b'{' if bytes.get(i + 1) == Some(&b'{') => i += 2,
                b'{' => {
                    interp_depth = 1;
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }
        if interp_depth > 0 {
            match b {
                b'{' => {
                    interp_depth += 1;
                    i += 1;
                    continue;
                }
                b'}' => {
                    interp_depth -= 1;
                    i += 1;
                    continue;
                }
                _ => {}
            }
        } else {
            match b {
                b'"' => {
                    in_string = true;
                    i += 1;
                    continue;
                }
                b'`' => {
                    let close = code[i + 1..].find(|c| c == '`' || c == '\n');
                    i = match close {
                        Some(off) if bytes[i + 1 + off] == b'`' => i + off + 2,
                        _ => i + 1,
                    };
                    continue;
                }
                _ => {}
            }
        }
        let starts_ident =
            (b.is_ascii_alphabetic() || b == b'_') && (i == 0 || !is_ident_byte(bytes[i - 1]));
        if !starts_ident {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_ident_byte(bytes[i]) {
            i += 1;
        }
        let word = &code[start..i];
        let Some(hidden) = renames.get(word) else {
            continue;
        };
        let names_the_definition = if word.starts_with(|c: char| c.is_ascii_uppercase()) {
            true
        } else {
            let line_start = code[..start].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let header_name = fn_section_name(&code[line_start..]);
            let mut before = code[..start].trim_end().rsplit(char::is_whitespace);
            let prev = before.next();
            bytes.get(i) == Some(&b'(')
                || (header_name.as_deref() == Some(word)
                    && code[line_start..start].trim_start().starts_with("## To "))
                || prev == Some("Call")
                || (prev == Some("to") && before.next() == Some("task"))
        };
        if names_the_definition {
            out.push_str(&code[copied..start]);
            out.push_str(hidden);
            copied = i;
        }
    }
    out.push_str(&code[copied..]);
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
    &PRELUDE_MODULES
}

/// Every identifier the prelude defines (across all modules) — derived from the modules.
pub fn prelude_vocabulary() -> Vec<String> {
    PRELUDE_NAMES
        .iter()
        .flat_map(|names| names.iter().map(|n| n.to_string()))
        .collect()
}

/// Module `idx`'s code as the prelude splices it, rebuilt from the source runs the build script found
/// ([`crate::stdlib_text::spliced_code`]).
fn spliced_module(idx: usize) -> String {
    let source = PRELUDE_MODULES[idx];
    let runs = PRELUDE_SPLICE[idx];
    let mut code = String::with_capacity(runs.iter().map(|&(s, e)| (e - s) as usize).sum());
    for &(s, e) in runs {
        code.push_str(&source[s as usize..e as usize]);
    }
    code
}

/// One run of [`apply_prelude`]'s output tied back to where it came from:
/// `user_start` is the byte offset of an untouched run of the caller's own
/// text, `None` marks spliced stdlib text. Downstream span translation
/// (deep check) inverts the prelude edit exactly through these runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreludeSegment {
    pub out_start: usize,
    pub len: usize,
    pub user_start: Option<usize>,
}

/// Prepend the stdlib modules a program actually uses. Returns the source
/// unchanged when the program references no stdlib vocabulary or opts out with
/// `## NoPrelude` (in which case the opt-out marker is stripped so it never
/// reaches the parser). This is the auto-import seam for both the interpreter and
/// the compiler.
pub fn apply_prelude(source: &str) -> std::borrow::Cow<'_, str> {
    apply_prelude_mapped(source).0
}

/// [`apply_prelude`] plus the exact edit it performed, as ordered output runs.
/// A consumer that parses the expanded text and reports spans against the
/// USER's buffer must translate through these segments rather than assume the
/// modules were prepended: they are spliced AFTER any leading title/comment
/// prose, and `## NoPrelude` REMOVES its marker lines.
pub fn apply_prelude_mapped(source: &str) -> (std::borrow::Cow<'_, str>, Vec<PreludeSegment>) {
    if let Some((stripped, segments)) = strip_no_prelude(source) {
        return (std::borrow::Cow::Owned(stripped), segments);
    }
    // "Declarer wins", PER NAME. A module is spliced when something needs one of its names that
    // the program does not declare itself; the program's declaration of any OTHER name of that
    // module never costs it the module. A name the program declares (a top-level `## To`/`## A`,
    // or a local closure — a call reaches a function before a local) is its own at every call it
    // writes, and the spliced library copy of that name is renamed out of the way
    // ([`rename_for_hygiene`]), so library code keeps calling the library's version. A name the
    // program binds as a plain VALUE coexists with the function and only stops counting as a use
    // unless it is called. The rule stays idempotent: a source already carrying a module's
    // definitions declares every name that module could be needed for, so it is left untouched
    // and the AOT hot path stays byte-identical.
    let locals = local_bindings(source);
    let user_defined: std::collections::HashSet<String> = defined_names(source)
        .into_iter()
        .chain(locals.callables)
        .collect();
    let user_values = locals.values;
    // Every scan below finds a name only as a whole word, so a name the program never writes as
    // one is decided without reading the program again.
    let words = whole_words(source);
    let demanded_by_program = |n: &str| {
        if !may_name(&words, n) {
            false
        } else if user_values.iter().any(|v| v == n) {
            references_call_only(source, n) || references_as_filter(source, n)
        } else {
            references(source, n)
        }
    };
    // TRANSITIVE references: a stdlib fn calling ANOTHER module's .lg fn (statistics.lg's
    // `fsumReal` → math.lg's `absReal`, or `geometricMeanReal` → `ln`/`exp`). A single
    // non-fixpoint pass over the PROGRAM's references alone splices statistics.lg but NOT math.lg,
    // leaving the transitive dependency an `Unknown function` on every tier. The fixpoint keeps
    // selecting a module whose name is referenced by the program OR by any already-selected module
    // (`PRELUDE_PULLS`, which the build script reads off each module's spliced code), until nothing
    // new is pulled — so a stdlib fn may freely call another module's fn. `builtin_backed` names
    // (sqrt, pow, …) resolve without a module, which is why only the non-builtin `.lg` deps
    // regressed.
    // Statement desugars reference stdlib names that never appear TEXTUALLY: `Solve … for x.`
    // lowers to `solveLinear`, `Minimize … over x.` to `minimizeQuadratic` (R4). The demand
    // scan must see through the sugar or every tier reports an unknown function.
    let mut virtual_refs: Vec<&'static str> = Vec::new();
    if source.lines().any(|l| l.trim_start().starts_with("Solve ")) {
        virtual_refs.push("solveLinear");
    }
    if source
        .lines()
        .any(|l| l.trim_start().starts_with("Minimize "))
    {
        virtual_refs.push("minimizeQuadratic");
    }
    // A `## Route GET /todos/:id` lowers to a dispatcher that matches the pattern and pulls the
    // named segment out — `matchRoute`/`routeParam`, from web.lg. Neither name appears in the
    // user's source: the header IS the reference. Without this the router is synthesized calling
    // functions the program never loaded, and every routed page dies with "Unknown function:
    // matchRoute" — a failure in generated code the author cannot see, naming a function they
    // never wrote. Literal-path routes need neither, so the trigger is a `:` segment.
    if source
        .lines()
        .any(|l| l.trim_start().starts_with("## Route ") && l.contains("/:"))
    {
        virtual_refs.push("matchRoute");
        virtual_refs.push("routeParam");
        virtual_refs.push("isIntText");
    }
    // ★ AND A `## Page /posts/:id` IS THE SAME SENTENCE ON THE OTHER TIER. Its `:name` segments
    // lower to the identical `routeParam(pattern, name, routePath())` binding, written by the
    // compiler from the header, so the demand scan cannot see it either. This is the third time
    // this exact shape has been paid for (`htmlEscape`, then `## Route`, now `## Page`): a
    // reference the COMPILER writes is invisible to a scan that reads the SOURCE.
    if source
        .lines()
        .any(|l| l.trim_start().starts_with("## Page ") && l.contains("/:"))
    {
        virtual_refs.push("matchRoute");
        virtual_refs.push("routeParam");
    }
    // ★ `Load "k" into f.` READS A TAG OFF THE STORED TEXT, through `cutAfter` — a call the
    // COMPILER writes from the sentence, so the scan that reads the author's tokens cannot see it.
    // Fifth instance of this exact shape (`htmlEscape`, `## Route`, `## Page`, the key modifiers,
    // now this): a reference the compiler writes must be declared at every stage that reasons
    // about reachability, or the program dies with `Unknown function` in generated code naming
    // something the author never wrote.
    // ★ `Add class …` / `Remove class …` DECIDE WHICH CLASS WORDS SURVIVE in compiled LOGOS, so
    // the lowering names two web.lg functions the author never wrote. Seventh instance of this
    // shape — a reference the compiler writes must be declared at every stage that reasons about
    // reachability, or the page dies with `Unknown function` naming something nobody typed.
    if source.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("Add class ") || l.starts_with("Remove class ")
    }) {
        virtual_refs.push("classListWith");
        virtual_refs.push("classListWithout");
    }
    if source.lines().any(|l| l.trim_start().starts_with("Load ")) {
        virtual_refs.push("cutAfter");
        // A `Bool` field decodes through convert.lg. `parseInt`/`parseFloat` are builtins and need
        // no declaring; this one is compiled LOGOS, so it does.
        virtual_refs.push("parseBool");
    }
    // ★ A PAGE RENDERS THROUGH vdom.lg: every component lowers to tape calls, a commit and the
    // sites it finds its nodes by — all written by the compiler, none in the author's source.
    if renders_a_page(source) {
        virtual_refs.push("vdCommit");
    }
    // A URL-valued attribute a paint computes goes through `safeUrl` — see [`computes_a_url`].
    if renders_a_page(source) && computes_a_url(source) {
        virtual_refs.push("safeUrl");
    }
    // A key modifier or a key trigger gates on which key was pressed, comparing case-insensitively
    // through `lower` — see [`uses_a_key_gate`].
    if uses_a_key_gate(source) {
        virtual_refs.push("lower");
    }
    if declares_a_convergent_shared_field(source) {
        virtual_refs.push("randomInt");
    }
    // A `## Session` synthesizes a global, a payload encoder and the two functions that move it
    // to and from a signed cookie — none of whose names the author ever types. The header IS the
    // reference, exactly as it is for `## Route`'s `matchRoute`.
    if source.lines().any(|l| l.trim_start() == "## Session") {
        for f in [
            "sessionSecret",
            "hexText",
            "sessionField",
            "sessionInt",
            "sessionBool",
            "signSession",
            "verifySession",
            "cookieOf",
            "withCookie",
            // The router checks CSRF itself for a state-changing request, and `csrf()` mints the
            // token a form carries — none of these names is typed by the author either.
            "csrfToken",
            "csrfValid",
            "randomSecret",
            "bodyParam",
            "headerOf",
            "upper",
        ] {
            virtual_refs.push(f);
        }
    }
    // EVERY dispatcher splits the URL it is handed into a path and a query before it matches
    // anything, so these are needed by a literal-path route, a lone `## Redirect` and a file
    // whose only server header is a middleware guard — all three synthesize the same walk.
    if source.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("## Route ")
            || l.starts_with("## Redirect ")
            || l.starts_with("## Before each request")
    }) {
        virtual_refs.push("pathOf");
        virtual_refs.push("queryString");
        // The synthesized handler reads the matched answer's `Location` out of its headers to tell
        // the router's own moved answer from a route that redirected itself, and `routeHeader`
        // reads any header by name. Neither name appears in the author's source.
        virtual_refs.push("headerIn");
        virtual_refs.push("queryParam");
        virtual_refs.push("urlDecode");
        virtual_refs.push("hexDigitValue");
        // The synthesized `Request -> Response` a bare `Serve on port N.` serves with: it rebuilds
        // a moved answer through `redirectTo`, so the reply constructors come with the table.
        virtual_refs.push("redirectTo");
        virtual_refs.push("permanentRedirectTo");
        virtual_refs.push("cutAfter");
        virtual_refs.push("bodyParam");

        virtual_refs.push("jsonIsNull");
    }
    // A derived `toJson_<T>` escapes its Text fields with `jsonQuote` (json.lg). The derive runs
    // AFTER parsing, so the name never appears in the user's source — calling the encoder is the
    // reference. Keyed on the CALL rather than on "this file declares a record", so a program
    // that never serialises anything does not drag in the JSON module.
    // A plain substring test, not `references`: that helper requires a WHOLE-WORD match, and
    // `toJson_` is by construction a PREFIX of the real name (`toJson_Todo`), so it could never
    // match one.
    // …and a `## Route` that answers a RECORD is wrapped in that record's encoder by the router
    // lowering, so `toJson_` never appears in the source either — the ROUTE is the reference. A
    // routed program is a server; pulling in the JSON module for one is the right trade against
    // a page that dies on its first typed response.
    // A TYPED FETCH (`Get "/api/todo" as Todo, then loaded.`) decodes its answer through the
    // derived decoder, so it needs the JSON module — and its `as` clause is the only reference,
    // since the decoder's name is written by the compiler rather than by the author.
    // ★ An `Upload` frames its body as `multipart/form-data` through these — names the compiler
    // writes, never the author, so the demand keys on the SENTENCE.
    let has_upload = source
        .lines()
        .any(|l| l.trim_start().starts_with("Upload "));
    if has_upload {
        virtual_refs.push("multipartBody");
        virtual_refs.push("multipartContentType");
        virtual_refs.push("multipartBoundary");
    }
    let has_typed_fetch = source.lines().any(|l| {
        let t = l.trim_start();
        ["Get ", "Post ", "Put ", "Patch ", "Delete ", "Upload "]
            .iter()
            .any(|v| t.starts_with(v))
            && t.contains(" as ")
            && t.contains("then")
    });
    // The DECODER pulls in more of json.lg than the encoder does — it parses the text and reads
    // fields back off the tree — so it names those entry points as its own demand.
    //
    // `Save`/`Load` (any field) and a `## Store` (any mode) route a `Seq`/record field through the
    // SAME derived codec (`parser/lgx_lower/{json,stores,elements}.rs`), and the encoder/decoder
    // names it calls (`toJsonSeq_<T>`, `fromJson_<T>`, …) are exactly as compiler-written as a
    // typed fetch's own — never present in the author's source for the JSON module to be pulled in
    // by the checks above. Keyed on the SENTENCE/HEADER (a plain scalar `Save`/`Load`/store field
    // never calls any of these, so this is conservative the same way `has_typed_fetch` already is)
    // rather than on whether a field is ACTUALLY a `Seq`/record, since that answer needs the parse
    // this prelude splice runs before.
    let has_save_or_load = source.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("Save ") || t.starts_with("Load ")
    });
    let has_store = source
        .lines()
        .any(|l| l.trim_start().starts_with("## Store"));
    if source.contains("toJson_")
        || source.contains("fromJson")
        || has_typed_fetch
        || has_save_or_load
        || has_store
        || source
            .lines()
            .any(|l| l.trim_start().starts_with("## Route "))
    {
        virtual_refs.push("jsonQuote");
    }
    if source.contains("fromJson") || has_typed_fetch || has_save_or_load || has_store {
        virtual_refs.push("jsonTryParse");
        virtual_refs.push("jsonSerialize");
        virtual_refs.push("jsonGet");
        virtual_refs.push("jsonInt");
        virtual_refs.push("jsonBool");
        virtual_refs.push("jsonString");
        // A `Seq` field's own encoder/decoder (`toJsonSeq_<T>`/`fromJsonSeq_<T>`) reads the
        // document as an ARRAY — the one json.lg entry point neither a bare typed fetch of a
        // record nor a route ever needed before a `Seq` payload existed on this path.
        virtual_refs.push("jsonArray");
    }
    let mut selected = vec![false; PRELUDE_MODULES.len()];
    loop {
        let mut added = false;
        // ★ `text_from_bytes` IS THE REFERENCE, and the author never types the name it needs.
        // The builtin must answer alike on every tier when the bytes are not valid UTF-8 — the
        // tree-walker and VM raise `text_from_bytes(): invalid UTF-8` and the AOT tier panics with
        // it — so the standalone wasm module has to trap too. It has no runtime to ask, so it CALLS
        // the rules, which are already written in LOGOS as `strings.lg`'s `validUtf8`. Fourth
        // instance of the compiler-written-reference shape after `htmlEscape`, `## Route` and
        // `## Page`; the first one keyed on a BUILTIN rather than on a header.
        //
        // Keyed on the selected modules too, not just the author's source: `encoding.lg`'s
        // `fromHex`, `json.lg`, `session.lg` and six others call `text_from_bytes` themselves, so a
        // program that only ever writes `fromHex(s)` reaches it without naming it once.
        if !virtual_refs.contains(&"validUtf8")
            && (source.contains("text_from_bytes")
                || (0..PRELUDE_MODULES.len())
                    .any(|j| selected[j] && PRELUDE_MENTIONS_TEXT_FROM_BYTES[j]))
        {
            virtual_refs.push("validUtf8");
            added = true;
        }
        for idx in 0..PRELUDE_MODULES.len() {
            if selected[idx] {
                continue;
            }
            let names = PRELUDE_NAMES[idx];
            // The two collisions hygiene cannot rename keep "declarer wins" at module grain: a
            // builtin-backed type (its declaration is stripped — the engine owns the name) and a
            // native (bound to its runtime implementation BY NAME on every tier).
            let natives = PRELUDE_NATIVES[idx];
            if names
                .iter()
                .any(|n| user_defined.contains(*n) && (builtin_backed(n) || natives.contains(n)))
            {
                continue;
            }
            let referenced = (0..PRELUDE_MODULES.len())
                .any(|j| selected[j] && PRELUDE_PULLS[j].contains(&(idx as u8)))
                || names.iter().any(|n| {
                    !builtin_backed(n)
                        && !user_defined.contains(*n)
                        && (demanded_by_program(n) || virtual_refs.contains(n))
                });
            if referenced {
                selected[idx] = true;
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let renames: std::collections::HashMap<String, String> = (0..PRELUDE_MODULES.len())
        .filter(|&idx| selected[idx])
        .flat_map(|idx| PRELUDE_NAMES[idx].iter())
        .filter(|n| user_defined.contains(**n))
        .map(|n| (n.to_string(), hygienic_name(n)))
        .collect();
    let needed: Vec<String> = (0..PRELUDE_MODULES.len())
        .filter(|&idx| selected[idx])
        .map(|idx| {
            let code = spliced_module(idx);
            if renames.is_empty() {
                code
            } else {
                rename_for_hygiene(&code, &renames)
            }
        })
        .collect();
    if needed.is_empty() {
        let segments = if source.is_empty() {
            Vec::new()
        } else {
            vec![PreludeSegment {
                out_start: 0,
                len: source.len(),
                user_start: Some(0),
            }]
        };
        (std::borrow::Cow::Borrowed(source), segments)
    } else {
        // Splice the modules in AFTER any leading title/comment prose (`# Title`,
        // `--` notes) and BEFORE the program's first `##` section. The lexer only
        // skips leading prose at the very start of a program, so prepending the
        // modules would push a user header into the middle of the assembled
        // source, where its `--` lines are a parse error.
        let insert_at = if source.starts_with("##") {
            0
        } else {
            source.find("\n##").map(|i| i + 1).unwrap_or(0)
        };
        let (head, tail) = source.split_at(insert_at);
        let mut out =
            String::with_capacity(source.len() + needed.iter().map(|m| m.len() + 2).sum::<usize>());
        out.push_str(head);
        for module in &needed {
            out.push_str(module);
            out.push_str("\n\n");
        }
        let spliced_len = out.len() - head.len();
        out.push_str(tail);
        let mut segments = Vec::with_capacity(3);
        if !head.is_empty() {
            segments.push(PreludeSegment {
                out_start: 0,
                len: head.len(),
                user_start: Some(0),
            });
        }
        segments.push(PreludeSegment {
            out_start: head.len(),
            len: spliced_len,
            user_start: None,
        });
        if !tail.is_empty() {
            segments.push(PreludeSegment {
                out_start: head.len() + spliced_len,
                len: tail.len(),
                user_start: Some(insert_at),
            });
        }
        (std::borrow::Cow::Owned(out), segments)
    }
}

/// Like [`references`] but ONLY counts a genuine CALL (`name(`), never a bare mention in a use
/// position (`to words`, a type annotation, prose). Drives the C reachability tree-shake: a
/// program whose *variable* shares a stdlib function's name (`Let words be …`, `Push x to words`)
/// must NOT drag that function in — on the C backend it would be emitted, collide, and even absorb
/// the caller's body. A function genuinely passed as a value (rare, higher-order) that this misses
/// surfaces as a loud `undefined reference`, never a silent miscompile.
fn references_call_only(source: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let sb = source.as_bytes();
    let mut from = 0;
    while let Some(off) = source[from..].find(name) {
        let start = from + off;
        let end = start + name.len();
        from = start + 1;
        if start > 0 && is_ident_byte(sb[start - 1]) {
            continue;
        }
        if end < sb.len() && is_ident_byte(sb[end]) {
            continue;
        }
        // Expression call `name(...)`.
        if end < sb.len() && sb[end] == b'(' {
            return true;
        }
        // Statement call `Call name with …` — `Call` is unambiguously an invocation (unlike
        // `to`/`of`/`the`/`a`, which also precede plain variables and caused the false trigger).
        if source[..start]
            .trim_end()
            .rsplit(char::is_whitespace)
            .next()
            == Some("Call")
        {
            return true;
        }
        // Concurrency spawn `Launch a task to name [with …]` — the function is named after `task to`,
        // so the two preceding words are unambiguously `task to`. Without this the C tree-shaker would
        // prune a spawned-only function (its body reached solely through `Launch`), leaving a dangling
        // `(void*)name` at the spawn site.
        let mut before = source[..start].trim_end().rsplit(char::is_whitespace);
        if before.next() == Some("to") && before.next() == Some("task") {
            return true;
        }
        // A function a real number is defined by, `the real number whose digits in base 10 are name` /
        // `the real number approximated by name`: the phrase reads the function whenever a refinement
        // of the number asks.
        if names_a_defining_function(source, start) {
            return true;
        }
        // A value handed to a function, `Show x to name.` / `Give x to name.`: the recipient closes a
        // sentence that opens with `Show` or `Give`. A bare `to` is not enough — `Set x to name.`
        // names a variable.
        let after = source[end..].trim_start_matches([' ', '\t']);
        if source[..start].trim_end().ends_with(" to")
            && (after.starts_with('.') || after.is_empty())
        {
            let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
            let opener = source[line_start..start].split_whitespace().next();
            if matches!(opener, Some("Show" | "Give")) {
                return true;
            }
        }
    }
    false
}

/// Whether the name at `start` closes a defined-real phrase outside a text literal: the words before it
/// end `the real number whose digits in base <k> are` or `the real number approximated by`.
fn names_a_defining_function(source: &str, start: usize) -> bool {
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let quotes = source[line_start..start].bytes().filter(|&b| b == b'"').count();
    if quotes % 2 == 1 {
        return false;
    }
    let mut before: Vec<&str> = source[line_start..start].split_whitespace().rev().take(9).collect();
    before.reverse();
    let opens = |the: &str, real: &str, number: &str| {
        the.eq_ignore_ascii_case("the") && real == "real" && number == "number"
    };
    match before.as_slice() {
        [.., the, real, number, "whose", "digits", "in", "base", _, "are"] => opens(the, real, number),
        [.., the, real, number, "approximated", "by"] => opens(the, real, number),
        _ => false,
    }
}

/// The functions of an assembled program that ARE the standard library's definitions: those whose
/// `## To` section is byte-identical to the section the library itself ships under that name.
///
/// This is provenance by content, and it is what licenses every name-keyed lowering of a library
/// function (the Rust tier's memchr `indexOf`, its AVX2 `trunc`, …): such a lowering encodes what
/// the LIBRARY's body computes, so it may replace a call only when the callee is that body. A
/// program's own function of the same name — the program's definition wins — never qualifies, and
/// neither does anything a program merely spells like the library. Sound by construction: a
/// program that copies a library section verbatim computes exactly what the lowering computes.
pub fn library_owned_functions(assembled: &str) -> std::collections::HashSet<String> {
    function_section_ranges(assembled)
        .into_iter()
        .filter(|(name, range)| library_section(name) == Some(&assembled[range.clone()]))
        .map(|(name, _)| name)
        .collect()
}

/// The `## To name` section the library ships, trimmed, when it ships one.
fn library_section(name: &str) -> Option<&'static str> {
    let at = LIBRARY_SECTIONS
        .binary_search_by(|&(n, ..)| n.cmp(name))
        .ok()?;
    let (_, module, start, end) = LIBRARY_SECTIONS[at];
    Some(&PRELUDE_MODULES[module as usize][start as usize..end as usize])
}

thread_local! {
    /// The functions of the program being compiled that ARE the standard library's own definitions
    /// ([`crate::loader::library_owned_functions`]). A lowering keyed on a library function's NAME
    /// (the Rust tier's memchr `indexOf`, native-width `rotateLeft`, AVX2 `trunc`; the interpreters' native `pbkdf2Sha256`) encodes what the library's body
    /// computes, so it may only replace a call whose callee is that body — never a program's own
    /// function spelled the same. Empty outside a [`LibraryProvenance`] scope: an entry that forgets
    /// to establish provenance loses the lowering, it can never hijack a program's function.
    static LIBRARY_FUNCTIONS: std::cell::RefCell<std::rc::Rc<std::collections::HashSet<String>>> =
        std::cell::RefCell::new(std::rc::Rc::new(std::collections::HashSet::new()));
}

/// Establishes, for the compilation or execution that runs while it lives, which functions of the program are the
/// standard library's definitions. Restores the enclosing provenance on drop, so nested and
/// sequential compiles on one thread never see each other's program.
pub struct LibraryProvenance {
    previous: std::rc::Rc<std::collections::HashSet<String>>,
    owned: std::rc::Rc<std::collections::HashSet<String>>,
}

impl LibraryProvenance {
    /// Enter the provenance of `assembled` — the exact text the codegen's AST was parsed from.
    pub fn of(assembled: &str) -> Self {
        let owned = std::rc::Rc::new(library_owned_functions(assembled));
        let previous =
            LIBRARY_FUNCTIONS.with(|c| std::mem::replace(&mut *c.borrow_mut(), owned.clone()));
        LibraryProvenance { previous, owned }
    }

    /// The program's functions that are the standard library's own definitions.
    pub fn functions(&self) -> &std::collections::HashSet<String> {
        &self.owned
    }
}

impl Drop for LibraryProvenance {
    fn drop(&mut self) {
        let previous = std::mem::take(&mut self.previous);
        LIBRARY_FUNCTIONS.with(|c| *c.borrow_mut() = previous);
    }
}

/// Is `name` the standard library's own definition in the program being compiled?
pub fn is_library_function(name: &str) -> bool {
    LIBRARY_FUNCTIONS.with(|c| c.borrow().contains(name))
}

/// Does `source` apply `name` as a template FILTER (`{price | money}`)? A filter is a call written
/// left to right, so it is a use of the function even where the same word is also a local value.
fn references_as_filter(source: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let sb = source.as_bytes();
    let mut from = 0;
    while let Some(off) = source[from..].find(name) {
        let start = from + off;
        let end = start + name.len();
        from = start + 1;
        if (start > 0 && is_ident_byte(sb[start - 1])) || (end < sb.len() && is_ident_byte(sb[end]))
        {
            continue;
        }
        let prefix = source[..start].trim_end();
        if prefix.ends_with('|') && !source[prefix.len()..start].contains('\n') {
            return true;
        }
    }
    false
}

/// Drop top-level `## To …` function definitions that are UNREACHABLE from the program's entry,
/// keeping every non-function section (`## A` structs, `## Main`, prose) and every function whose
/// name is transitively called. This is the C backends' tree-shake: a flat C namespace can't
/// tolerate emitting an *unused* stdlib function that either collides with a libc name or exercises
/// a C-codegen gap the function is never actually reached through (the Rust backend tolerates both,
/// so this is C-only).
///
/// Reachability is seeded from the entry sections of `spliced` (the prelude-spliced source) and from
/// every function a post-parse lowering calls — a `## Route` dispatcher's `pathOf`, a derived codec's
/// `jsonQuote` — which [`synthesized_prelude_roots`] reads off `user_source`, the author's own text.
/// Those calls are written by the compiler, so no scan of the source can see them; a pruner that
/// dropped them would leave the lowering calling a function that is gone, which the bytecode compiler
/// turns into an `Unknown function` failure at run time.
pub fn keep_reachable_functions(user_source: &str, spliced: &str) -> String {
    // 1. Split into `## `-delimited sections, preserving exact text + order.
    let mut sections: Vec<String> = Vec::new();
    for line in spliced.split_inclusive('\n') {
        if line.starts_with("## ") || sections.is_empty() {
            sections.push(String::new());
        }
        sections.last_mut().unwrap().push_str(line);
    }
    let names: Vec<Option<String>> = sections.iter().map(|s| fn_section_name(s)).collect();
    let fn_names: Vec<&str> = names.iter().filter_map(|n| n.as_deref()).collect();
    if fn_names.is_empty() {
        return spliced.to_string();
    }
    // 2. Seed reachable from every NON-function section (Main, structs, prose).
    let root: String = sections
        .iter()
        .zip(&names)
        .filter(|(_, n)| n.is_none())
        .map(|(s, _)| s.as_str())
        .collect();
    let synthesized = synthesized_prelude_roots(user_source);
    let mut reachable: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for &name in &fn_names {
        if references_call_only(&root, name) || synthesized.iter().any(|s| s == name) {
            reachable.insert(name);
        }
    }
    // 3. Fixpoint: a reachable function pulls in every function its body references.
    loop {
        let mut added = false;
        for (s, n) in sections.iter().zip(&names) {
            let Some(nm) = n.as_deref() else { continue };
            if !reachable.contains(nm) {
                continue;
            }
            for &cand in &fn_names {
                if !reachable.contains(cand) && references_call_only(s, cand) {
                    reachable.insert(cand);
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }
    // 4. Keep non-function sections + reachable function sections, in original order.
    let mut out = String::with_capacity(spliced.len());
    for (s, n) in sections.iter().zip(&names) {
        match n {
            Some(nm) if !reachable.contains(nm.as_str()) => {}
            _ => out.push_str(s),
        }
    }
    out
}

/// Drop ONLY the auto-imported stdlib PRELUDE functions that are never reachable, keeping EVERY
/// user-defined function untouched. This is the chip-encoder analog of [`keep_reachable_functions`], but
/// safe for LOGOS's non-`name(` call forms: `apply_prelude` splices a WHOLE module when the program mentions
/// any of its names, so an UNCALLED prelude helper that reaches a host builtin (`parseFloat`, `format`, …)
/// would make the flat chip image reject even though it never runs. This prunes those dead prelude helpers
/// so the encoder lowers only what the program executes — with two safety differences from the C tree-shake:
///   • it NEVER drops a user function (the prune set is exactly the prelude vocabulary), so a user procedure
///     invoked by a LOGOS call form the call-only scan misses (`Launch a task to worker`, higher-order
///     `apply(f, …)`) is always kept;
///   • reachability uses the PERMISSIVE `references` (any use position — the same predicate `apply_prelude`
///     splices on), so a prelude function reached via `to <fn>` / `of <fn>` / a value position is retained.
/// The output is byte-identical to the input when nothing is pruned (no prelude, or all reachable), so the
/// AOT/other hot paths are untouched. Over-pruning can only ever surface as a LOUD undefined reference.
pub fn prune_unreachable_prelude(source: &str) -> String {
    prune_unreachable_prelude_mapped(source).0
}

/// [`prune_unreachable_prelude`] told which names the USER's own (pre-prelude) source declares, so a
/// name the stdlib also has is recognized as the user's and kept. Callers that have the original
/// source should prefer this; the bare form behaves as before.
pub fn prune_unreachable_prelude_from(user_source: &str, spliced: &str) -> String {
    prune_unreachable_prelude_mapped_from(user_source, spliced).0
}

/// [`prune_unreachable_prelude_from`] with the byte ranges of `spliced` it kept, as
/// [`prune_unreachable_prelude_mapped`] reports them — for a caller that maps offsets in the pruned
/// text back, and must still prune exactly as a build does.
pub fn prune_unreachable_prelude_mapped_from(
    user_source: &str,
    spliced: &str,
) -> (String, Vec<(usize, usize)>) {
    let mut declared = defined_names(user_source);
    // ★ ROOTS A POST-PARSE LOWERING WILL CALL, which the source text cannot show.
    //
    // `apply_prelude` already understands these — a page demands vdom.lg's calls, a `## Route`
    // demands the router's helpers — but this pruner runs on the SOURCE and keeps only what the
    // source NAMES. So it dropped the very function the demand had just pulled in, and the
    // lowering then emitted a call to a name that was gone. (`Unknown function: htmlEscape`,
    // from AOT-Rust; the tree-walker survived it because it resolves lazily, which is exactly
    // how a gap like this stays hidden on the reference tier.)
    //
    // Same shape as the demand itself, one stage later: a construct whose reference to a stdlib
    // function is written BY THE COMPILER has to declare that reference at every stage that
    // reasons about reachability.
    declared.extend(synthesized_prelude_roots(user_source));
    let refs: Vec<&str> = declared.iter().map(|s| s.as_str()).collect();
    prune_unreachable_prelude_mapped_with(spliced, &refs)
}

/// Does a `## Store … shared on "…"` declare, in its own `State:`, a field that converges as a CRDT
/// (`Let likes: Tally be 0.`)? Such a store draws its page's replica id through `randomInt` — a call
/// the lowering writes (`lgx_lower::shared`), once per page and only for a converging store, so a
/// shared store of plain values, a store that is not shared, and a convergent field elsewhere demand
/// nothing. The names are the ones `shared_field_kind` converges.
fn declares_a_convergent_shared_field(source: &str) -> bool {
    const CONVERGENT: [&str; 9] = [
        "Tally",
        "PNCounter",
        "ConvergentCount",
        "GCounter",
        "SharedSet",
        "ORSet",
        "SharedSet_AddWins",
        "SharedSequence",
        "RGA",
    ];
    let mut in_shared_store = false;
    for line in source.lines() {
        let t = line.trim_start();
        if t.starts_with("## ") {
            in_shared_store = t.starts_with("## Store ") && t.contains(" shared on ");
            continue;
        }
        if !in_shared_store {
            continue;
        }
        let Some(binding) = t.strip_prefix("Let ") else {
            continue;
        };
        let binding = binding.strip_prefix("mutable ").unwrap_or(binding);
        let Some((_, annotation)) = binding.split_once(':') else {
            continue;
        };
        let head = annotation.split_whitespace().next().unwrap_or("");
        if CONVERGENT.contains(&head) {
            return true;
        }
    }
    false
}

/// Prelude functions a LOWERING will call even though the source never names them.
///
/// Does this source carry a KEY MODIFIER (`on-keydown.enter={…}`) or a key TRIGGER
/// (`When the user presses Escape …:`)?
///
/// Both lower to a case-insensitive comparison against the key, which is `lower` from
/// `strings.lg` — a call the COMPILER writes from the modifier, so the demand scan that reads the
/// author's source cannot see it. Fourth time this exact shape has been paid for: `htmlEscape`,
/// then `## Route`'s `matchRoute`, then `## Page`'s `routeParam`, now this.
///
/// `.prevent`/`.stop`/`.once`/`.capture`/`.passive` are the BEHAVIOURAL modifiers and name no key,
/// so a page using only those needs nothing extra — keeping the pruned prelude of every existing
/// page exactly as it was.
/// Whether `source` declares something drawn through vdom.lg — a component, a page, an error page,
/// a `-> View` function or route — or asks for markup (`html of`).
fn renders_a_page(source: &str) -> bool {
    source.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("## Component ")
            || l.starts_with("## Page ")
            || l == "## Page"
            || l.starts_with("## Error page ")
            || ((l.starts_with("## To ") || l.starts_with("## Route ")) && l.contains("-> View"))
    }) || source
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|w| w[0].eq_ignore_ascii_case("html") && w[1] == "of")
}

/// Whether a view writes a URL-valued attribute (`href`, `src`, … — `lgx::html::URL_ATTRIBUTES`)
/// whose value is computed: a hole (`href={link}`) or a quoted value holding one
/// (`href="/p/{id}"`). The paint checks such a value's scheme with `safeUrl` when it runs; a
/// written URL is checked when the view is compiled and needs nothing at run time.
fn computes_a_url(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    logicaffeine_language::lgx::html::URL_ATTRIBUTES
        .iter()
        .any(|attr| {
            let needle = format!("{attr}=");
            lower.match_indices(&needle).any(|(at, _)| {
                let before = at.checked_sub(1).map(|i| lower.as_bytes()[i]);
                let written_alone = before.is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'-');
                let value = &lower[at + needle.len()..];
                written_alone
                    && match value.as_bytes().first() {
                        Some(b'{') => true,
                        Some(b'"') => value[1..]
                            .split('"')
                            .next()
                            .is_some_and(|quoted| quoted.contains('{')),
                        _ => false,
                    }
            })
        })
}

fn uses_a_key_gate(source: &str) -> bool {
    const BEHAVIOURAL: &[&str] = &["prevent", "stop", "once", "capture", "passive"];
    for line in source.lines() {
        if line.contains("presses ") {
            return true;
        }
        let mut rest = line;
        while let Some(i) = rest.find("on-") {
            rest = &rest[i + 3..];
            // The directive runs to `=` or to whitespace; a `.` inside it introduces a modifier.
            let end = rest.find(['=', ' ', '\t', '>']).unwrap_or(rest.len());
            let directive = &rest[..end];
            if let Some(dot) = directive.find('.') {
                let modifier = directive[dot + 1..].split('.').next().unwrap_or("");
                if !modifier.is_empty() && !BEHAVIOURAL.contains(&modifier) {
                    return true;
                }
            }
        }
    }
    false
}

/// Kept beside the demand rules in `apply_prelude` deliberately: the two answer the same question
/// ("what does this program actually reach?") at different stages, and they drift apart the
/// moment they live apart.
fn synthesized_prelude_roots(source: &str) -> Vec<String> {
    let mut roots = Vec::new();
    if source
        .lines()
        .any(|l| l.trim_start().starts_with("Upload "))
    {
        roots.push("multipartBody".to_string());
        roots.push("multipartContentType".to_string());
        roots.push("multipartBoundary".to_string());
    }
    // ★ A DERIVED CODEC'S CALLS ARE WRITTEN BY THE COMPILER. `toJson_Todo` and `fromJson_Todo`
    // are emitted after parsing and reach into json.lg for the parser and the accessors — names
    // the author's source never contains, so the pruner (which keeps only what the SOURCE names)
    // threw them away and the program died with `Unknown function: jsonTryParse`.
    //
    // ★ AND IT ONLY BITES ON A BUILD. The tree-walker resolves lazily, so every single-file lock
    // passed while `largo build` and `largo check` refused the same program — the second time
    // this exact shape has cost real time (the first was `htmlEscape`), which is why the demand
    // rules and these roots live side by side: they answer one question at two stages.
    if source.contains("toJson") || source.contains("fromJson") {
        for name in [
            "jsonQuote",
            "jsonTryParse",
            "jsonGet",
            "jsonString",
            "jsonInt",
            "jsonBool",
            "jsonArray",
            "jsonSerialize",
            "jsonIsNull",
        ] {
            roots.push(name.to_string());
        }
    }
    // `html of <Component>` lowers to a server-render function whose every hole is escaped, and
    // whose URL-valued attributes additionally go through `safeUrl` — escaping neutralises markup,
    // and a `javascript:` URL contains none.
    // The other half of the `Load` demand above — the pruner keeps only what the SOURCE names,
    // and `cutAfter` is named by the lowering rather than by the author.
    // The pruner half of the class-list demand above.
    if source.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("Add class ") || l.starts_with("Remove class ")
    }) {
        roots.push("classListWith".to_string());
        roots.push("classListWithout".to_string());
    }
    if source.lines().any(|l| l.trim_start().starts_with("Load ")) {
        roots.push("cutAfter".to_string());
        roots.push("parseBool".to_string());
    }
    // A `## Route` is wrapped by the router walk, which matches paths, reads parameters and
    // quotes a JSON answer.
    if source
        .lines()
        .any(|l| l.trim_start().starts_with("## Route "))
    {
        roots.push("matchRoute".to_string());
        roots.push("routeParam".to_string());
        roots.push("isIntText".to_string());
        roots.push("jsonQuote".to_string());
    }
    // A `## Page /posts/:id` binds its parameters through the same two helpers — see the
    // matching `virtual_refs` block. The demand pulls them in; without this the pruner, which
    // keeps only what the SOURCE names, deletes them again one stage later.
    if source
        .lines()
        .any(|l| l.trim_start().starts_with("## Page ") && l.contains("/:"))
    {
        roots.push("matchRoute".to_string());
        roots.push("routeParam".to_string());
    }
    // The page's vdom.lg calls — see the matching `virtual_refs` block.
    if renders_a_page(source) {
        roots.extend(
            logicaffeine_language::parser::lgx_lower::VDOM_CALLS
                .iter()
                .map(|f| f.to_string()),
        );
    }
    // The pruner half of the `safeUrl` demand above.
    if renders_a_page(source) && computes_a_url(source) {
        roots.push("safeUrl".to_string());
    }
    // The key gate's `lower` — see the matching `virtual_refs` block.
    if uses_a_key_gate(source) {
        roots.push("lower".to_string());
    }
    if declares_a_convergent_shared_field(source) {
        roots.push("randomInt".to_string());
    }
    // The session's own machinery — see the matching `virtual_refs` block.
    if source.lines().any(|l| l.trim_start() == "## Session") {
        for f in [
            "sessionSecret",
            "randomSecret",
            "hexText",
            "unhexText",
            "sessionField",
            "sessionInt",
            "sessionBool",
            "signSession",
            "verifySession",
            "cookieOf",
            "withCookie",
            "cookie",
            "isCookieSafe",
            "macHex",
            "toHexInt",
            "bytesToHex",
            "bytesFromHex",
            "isIntText",
            "csrfToken",
            "csrfValid",
            "bodyParam",
            "headerOf",
            "upper",
            "randomInt",
            "split",
            "parseInt",
        ] {
            roots.push(f.to_string());
        }
    }
    // The URL split, and the query reader a parameter with no `:name` slot binds through.
    if source.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("## Route ")
            || l.starts_with("## Redirect ")
            || l.starts_with("## Before each request")
    }) {
        roots.push("pathOf".to_string());
        roots.push("queryString".to_string());
        roots.push("headerIn".to_string());
        roots.push("queryParam".to_string());
        roots.push("urlDecode".to_string());
        roots.push("hexDigitValue".to_string());
        roots.push("redirectTo".to_string());
        roots.push("permanentRedirectTo".to_string());
        roots.push("cutAfter".to_string());
        roots.push("bodyParam".to_string());

        roots.push("jsonIsNull".to_string());
    }
    roots
}

/// [`prune_unreachable_prelude`] plus the KEPT byte ranges of the INPUT (merged and
/// in order), so a consumer that parses the pruned text can remap an offset back to
/// the original source. Deep check uses this to drop EXACTLY what `largo build` drops
/// (it prunes the same way before it parses) while keeping every rustc finding mapped
/// to the user's own line. The ranges partition the kept output: concatenating
/// `source[s..e]` for each `(s, e)` reproduces the pruned string.
pub fn prune_unreachable_prelude_mapped(source: &str) -> (String, Vec<(usize, usize)>) {
    prune_unreachable_prelude_mapped_with(source, &[])
}

/// [`prune_unreachable_prelude_mapped`] with the names the user's own source declares, which are
/// never eligible for pruning (see there).
pub fn prune_unreachable_prelude_mapped_with(
    source: &str,
    user_declared: &[&str],
) -> (String, Vec<(usize, usize)>) {
    // The prelude's function names — the ONLY sections eligible for pruning.
    let mut prelude_fns: std::collections::HashSet<String> =
        PRELUDE_FUNCTIONS.iter().map(|n| n.to_string()).collect();
    if prelude_fns.is_empty() {
        return (source.to_string(), vec![(0, source.len())]);
    }
    // A prelude NAME the USER also declares is the user's own function, not the stdlib's, and must
    // never be pruned: `apply_prelude` does not prepend a module whose names the source already
    // defines, so when the user writes `## To native args -> Seq of Text` the only section carrying
    // that name IS theirs. Pruning it as unreachable stdlib deleted a declaration the program needs —
    // a native nothing calls yet still has to reach codegen, because it is the program's own
    // declaration of its own native. `user_declared` is empty for callers that do not have the
    // pre-prelude source, which restores the previous behaviour exactly.
    for declared in user_declared {
        prelude_fns.remove(*declared);
    }
    if prelude_fns.is_empty() {
        return (source.to_string(), vec![(0, source.len())]);
    }

    // Split into `## `-delimited sections (exact text + order), classify each by its function name.
    let mut sections: Vec<String> = Vec::new();
    for line in source.split_inclusive('\n') {
        if line.starts_with("## ") || sections.is_empty() {
            sections.push(String::new());
        }
        sections.last_mut().unwrap().push_str(line);
    }
    let names: Vec<Option<String>> = sections.iter().map(|s| fn_section_name(s)).collect();

    // A section is a PRUNABLE prelude function iff its name is in the prelude vocabulary; every other section
    // (Main, structs, prose, and every USER function) is a permanent ROOT.
    let is_prelude_fn = |n: &Option<String>| {
        n.as_deref()
            .map(|nm| prelude_fns.contains(nm))
            .unwrap_or(false)
    };

    // Seed reachability from the ROOTS (everything not a prunable prelude function), then fixpoint-expand over
    // the prelude functions, using the PERMISSIVE any-position `references` so a `to <fn>` call is seen.
    let root: String = sections
        .iter()
        .zip(&names)
        .filter(|(_, n)| !is_prelude_fn(n))
        .map(|(s, _)| s.as_str())
        .collect();
    let prelude_names: Vec<&str> = names
        .iter()
        .zip(&sections)
        .filter(|(n, _)| is_prelude_fn(n))
        .filter_map(|(n, _)| n.as_deref())
        .collect();
    let mut reachable: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for &name in &prelude_names {
        if references(&root, name) {
            reachable.insert(name);
        }
    }
    loop {
        let mut added = false;
        // ★ THE ONE EDGE THE TEXT CANNOT SHOW: `text_from_bytes` reaches `validUtf8`. The wasm
        // backend lowers that builtin into a call to the stdlib validator so a browser traps on
        // invalid UTF-8 exactly as the reference tier does (`vm/wasm/module.rs`), and the CALLER
        // may be a spliced module rather than the author's own source — `encoding.lg`'s `fromHex`
        // reaches it without the program naming it once. Kept beside the demand rule in
        // `apply_prelude`, which answers the same question one stage earlier.
        if prelude_names.contains(&"validUtf8")
            && !reachable.contains("validUtf8")
            && (references(&root, "text_from_bytes")
                || sections.iter().zip(&names).any(|(s, n)| {
                    n.as_deref()
                        .map(|nm| reachable.contains(nm))
                        .unwrap_or(false)
                        && references(s, "text_from_bytes")
                }))
        {
            reachable.insert("validUtf8");
            added = true;
        }
        for (s, n) in sections.iter().zip(&names) {
            let Some(nm) = n.as_deref() else { continue };
            if !reachable.contains(nm) {
                continue;
            }
            for &cand in &prelude_names {
                if !reachable.contains(cand) && references(s, cand) {
                    reachable.insert(cand);
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }

    // Keep every non-prelude-function section + every REACHABLE prelude function, in
    // original order — recording each KEPT byte range in the INPUT (merging adjacent
    // ones) so a consumer can map an offset in the pruned text back to `source`.
    let mut out = String::with_capacity(source.len());
    let mut kept: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0usize;
    for (s, n) in sections.iter().zip(&names) {
        let seg_len = s.len();
        let drop = is_prelude_fn(n) && !reachable.contains(n.as_deref().unwrap());
        if !drop {
            match kept.last_mut() {
                Some(last) if last.1 == pos => last.1 = pos + seg_len,
                _ => kept.push((pos, pos + seg_len)),
            }
            out.push_str(s);
        }
        pos += seg_len;
    }
    (out, kept)
}

/// If `source` has a `## NoPrelude` opt-out line, return the source with that
/// line removed plus the surviving runs as [`PreludeSegment`]s; otherwise
/// `None`. Byte-preserving: every kept line comes through untouched (CRLF and
/// the trailing newline included), so the segments tie each output run to an
/// identical run of the user's own bytes.
fn strip_no_prelude(source: &str) -> Option<(String, Vec<PreludeSegment>)> {
    if !source.lines().any(|l| l.trim() == "## NoPrelude") {
        return None;
    }
    let mut out = String::with_capacity(source.len());
    let mut segments: Vec<PreludeSegment> = Vec::new();
    let mut user_pos = 0usize;
    for line in source.split_inclusive('\n') {
        if line.trim() == "## NoPrelude" {
            user_pos += line.len();
            continue;
        }
        match segments.last_mut() {
            Some(seg) if seg.user_start.map(|us| us + seg.len) == Some(user_pos) => {
                seg.len += line.len();
            }
            _ => segments.push(PreludeSegment {
                out_start: out.len(),
                len: line.len(),
                user_start: Some(user_pos),
            }),
        }
        out.push_str(line);
        user_pos += line.len();
    }
    Some((out, segments))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/loader/tests.rs"]
mod tests;
