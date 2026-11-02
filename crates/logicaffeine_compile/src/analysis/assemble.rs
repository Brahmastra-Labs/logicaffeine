//! Source-level assembly of a multi-file / multi-directory LOGOS project.
//!
//! A project is ONE compilation unit. [`assemble_project`] resolves the entry
//! file's explicit imports (markdown links in its abstract), transitively loads
//! every reachable module, and splices their code into a single combined source
//! string — the same string-level seam the stdlib prelude already uses
//! ([`apply_prelude`](crate::loader::apply_prelude)). Because every execution
//! tier (AOT, interpreter, VM, wasm, isa, C) re-parses from a source string, one
//! assembled string feeds them all: the imported functions and types reach the
//! parser, the call graph, and codegen exactly as if the program had been
//! written in a single file.
//!
//! ## Ordering
//!
//! Modules are collected in **post-order DFS** over the import graph, so a
//! module's dependencies are spliced before it and the entry's own body (with
//! its `## Main`) comes last. The tree-walking interpreter registers each
//! `## To` definition as it walks and has no hoisting pass, so "definitions
//! before the Main that uses them" must hold structurally — post-order gives it.
//!
//! ## Cycle & diamond safety
//!
//! Collection dedups by **canonical path** through a `visited` set, seeded with
//! the entry. Mutual imports (`A` uses `B`, `B` uses `A`) are legal — required
//! for cross-file mutual recursion — and contribute each module's code exactly
//! once. A module that imports the entry back never re-splices the entry's
//! `## Main`.
//!
//! ## Foreign `## Main`
//!
//! Only the ENTRY's `## Main` survives; an imported module contributes its
//! functions and type definitions but never a `## Main` (stripped by
//! `strip_main_block`). This preserves the single-entry-Main contract every
//! tier relies on.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use logicaffeine_base::{Interner, Symbol};
use logicaffeine_language::ast::stmt::Stmt;
use logicaffeine_language::token::{RawKind, Token, TokenType};

use crate::analysis::{first_section_header, misplaced_imports, scan_dependencies};
use crate::loader::{
    defined_names_with_visibility, function_header, module_code, strip_note_blocks, DepRoot, Loader,
};
use crate::Lexer;
use logicaffeine_language::analysis::SelectedName;

/// One assembled compilation unit: the combined source plus the metadata later
/// passes need for qualified-name resolution, collision diagnostics, and
/// cross-file error attribution.
pub struct AssembledProgram {
    /// The single combined source string: the entry (with its leading prose and
    /// `## Main`) with every imported module's code spliced in before its first
    /// `##` section, dependencies first. Fed verbatim to the existing
    /// lex/parse/codegen seam.
    pub source: String,
    /// Which module defines each bare name — the ground truth for the
    /// hard-error on an ambiguous bare reference and for qualified→canonical
    /// resolution.
    pub provenance: Provenance,
    /// Maps a byte offset in [`source`](Self::source) back to the file it came
    /// from, so a diagnostic in an imported module reports against THAT file
    /// rather than the combined blob.
    pub source_map: SourceMap,
    /// Per-file name scope — what each file may reference (its own definitions
    /// plus the exports of the modules it explicitly imports). Drives
    /// [`check_scopes`], which forbids transitive leakage.
    pub scope: ScopeInfo,
}

/// Per-file lexical scope for the whole project: what each file is allowed to
/// reference. A file sees its own definitions and the exports of the modules it
/// *directly* imports — never anything reached only transitively.
#[derive(Debug, Default)]
pub struct ScopeInfo {
    /// Module canonical path → the names in scope for code in that file (own
    /// definitions ∪ directly-imported modules' exports).
    in_scope: HashMap<PathBuf, HashSet<String>>,
    /// Every name that is *some* user module's export — the only names a scope
    /// violation can be reported for (builtins/stdlib/unknowns are never
    /// flagged, so the check cannot false-positive a valid program).
    all_exports: HashSet<String>,
    /// Name → the module that defines it (for the diagnostic).
    defn_file: HashMap<String, PathBuf>,
    /// Names declared `private` by their module — reachable only from the module
    /// that owns them. A reference from any other file is a hard error.
    private_names: HashSet<String>,
    /// Original private name → the module that owns it (for the diagnostic).
    private_owner: HashMap<String, PathBuf>,
    /// Per-name import aliases (`Import double as dbl from Util.`): surface alias
    /// → real exported name. Rewritten to the real name before every scope check
    /// and codegen so a `dbl` reference resolves to `double`.
    selective_aliases: HashMap<String, String>,
    /// The entry file — the home of the top-level `## Main` body.
    entry: PathBuf,
}

impl ScopeInfo {
    /// The project's entry file (home of the top-level `## Main`).
    pub fn entry(&self) -> &Path {
        &self.entry
    }

    /// Per-name import aliases: surface alias → real exported name.
    pub fn selective_aliases(&self) -> &HashMap<String, String> {
        &self.selective_aliases
    }
}

/// Enforce per-file scoping over the assembled program: a reference to a name
/// that is another user module's export, from a file that neither defines nor
/// imports it, is a hard error (no transitive leakage). Builtins, stdlib, and a
/// file's own/imported names are always fine, so a valid program never trips.
pub fn check_scopes(
    stmts: &[Stmt<'_>],
    scope: &ScopeInfo,
    interner: &Interner,
) -> Result<(), String> {
    use crate::analysis::callgraph::{collect_calls_from_stmts, collect_type_refs_from_stmts};

    // Each top-level DEFINITION's body is checked against the scope of the file that defines
    // it — both the functions it CALLS and the types it REFERENCES (constructions,
    // annotations) — since a private name of either kind is reachable only from its owning
    // module. `defining_module_key` says which statements carry their own scope this way;
    // everything else falls to the entry-file bucket below, exactly like plain `## Main` code.
    for stmt in stmts {
        let Some(key) = defining_module_key(stmt, interner) else {
            continue;
        };
        let Some(file) = scope.defn_file.get(&key) else {
            continue;
        };
        let mut refs = HashSet::new();
        if let Stmt::FunctionDef { body, .. } = stmt {
            collect_calls_from_stmts(body, &mut refs);
            collect_type_refs_from_stmts(body, &mut refs);
        } else {
            let one = std::slice::from_ref(stmt);
            collect_calls_from_stmts(one, &mut refs);
            collect_type_refs_from_stmts(one, &mut refs);
        }
        check_call_set(&refs, file, scope, interner)?;
    }
    // The top-level `## Main` body belongs to the entry file — everything
    // `defining_module_key` does not itself attribute.
    let mut main_refs = HashSet::new();
    for stmt in stmts {
        if defining_module_key(stmt, interner).is_none() {
            collect_calls_from_stmts(std::slice::from_ref(stmt), &mut main_refs);
            collect_type_refs_from_stmts(std::slice::from_ref(stmt), &mut main_refs);
        }
    }
    check_call_set(&main_refs, &scope.entry, scope, interner)
}

/// The key `scope.defn_file` records for a top-level `Stmt`'s OWN definition, when it has one —
/// the file whose scope its body must be checked against, rather than the entry's. `None` means
/// the statement is not itself a named definition another module could import; its calls belong
/// to whichever surrounding block is checked (today, always the entry's `## Main`).
///
/// EXHAUSTIVE, no wildcard: a new `Stmt` variant must say which bucket it belongs to here, so it
/// can never silently fall into the entry bucket the way `Stmt::ComponentDef` once did — every
/// call inside a `## Component`'s `Actions:`/`View:` (a `{greeting(count)}` interpolation, an
/// event handler's own body) was checked against the ENTRY file's imports instead of the
/// component's OWN module's, because the LGX parser desugars a component to a *structured*
/// `ComponentDef` node, never the retired dialect's flat `FunctionDef` this dispatch used to
/// require.
///
/// `## Page`/`## Store`/`## Error page` share `ComponentDef`'s shape but are not yet covered here:
/// their render function (`__render_<name>`, `lgx_lower::render_fn_name`) is real, but
/// `record_provenance`'s text scan (`component_render_name`) only recognizes a literal
/// `"## Component "` header, so `defn_file` never gains an entry for them — the lookup above
/// finds nothing and the call falls through to the entry bucket, same as before this fix (a
/// documented gap, not a silent one). `## Route`/`## Directive`/
/// `## Before each request`/`## Redirect` are deliberately `None`: none of them exports a NAME
/// another module calls (a route answers an HTTP method+path, never a LOGOS call site), so the
/// misattribution this fixes does not apply to them.
fn defining_module_key(stmt: &Stmt<'_>, interner: &Interner) -> Option<String> {
    match stmt {
        Stmt::FunctionDef { name, .. } => Some(interner.resolve(*name).to_string()),
        Stmt::ComponentDef(def) => Some(format!("__render_{}", interner.resolve(def.name))),
        Stmt::Let { .. }
        | Stmt::Set { .. }
        | Stmt::Swap { .. }
        | Stmt::Call { .. }
        | Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::Repeat { .. }
        | Stmt::Return { .. }
        | Stmt::Break
        | Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::RuntimeAssert { .. }
        | Stmt::Fail { .. }
        | Stmt::Give { .. }
        | Stmt::Show { .. }
        | Stmt::SetField { .. }
        | Stmt::StructDef { .. }
        | Stmt::TestDef { .. }
        | Stmt::Expect { .. }
        | Stmt::ExpectFail { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::AdvanceTime { .. }
        | Stmt::GivenFile { .. }
        | Stmt::Observe { .. }
        | Stmt::During { .. }
        | Stmt::ForAll { .. }
        | Stmt::Inspect { .. }
        | Stmt::Push { .. }
        | Stmt::Pop { .. }
        | Stmt::Add { .. }
        | Stmt::Remove { .. }
        | Stmt::SetIndex { .. }
        | Stmt::Splice { .. }
        | Stmt::Reversibly { .. }
        | Stmt::Zone { .. }
        | Stmt::Concurrent { .. }
        | Stmt::Parallel { .. }
        | Stmt::ReadFrom { .. }
        | Stmt::WriteFile { .. }
        | Stmt::Spawn { .. }
        | Stmt::SendMessage { .. }
        | Stmt::AwaitMessage { .. }
        | Stmt::StreamMessage { .. }
        | Stmt::MergeCrdt { .. }
        | Stmt::JoinCrdt { .. }
        | Stmt::IncreaseCrdt { .. }
        | Stmt::DecreaseCrdt { .. }
        | Stmt::AppendToSequence { .. }
        | Stmt::ResolveConflict { .. }
        | Stmt::Check { .. }
        | Stmt::Listen { .. }
        | Stmt::Serve { .. }
        | Stmt::ConnectTo { .. }
        | Stmt::LetPeerAgent { .. }
        | Stmt::Sleep { .. }
        | Stmt::Sync { .. }
        | Stmt::Ui(..)
        | Stmt::Fetch { .. }
        | Stmt::Replay { .. }
        | Stmt::ServeFetches
        | Stmt::GivenFetch { .. }
        | Stmt::DirectiveDef { .. }
        | Stmt::RouteDef { .. }
        | Stmt::MiddlewareDef { .. }
        | Stmt::RedirectDef { .. }
        | Stmt::Mount { .. }
        | Stmt::LaunchTask { .. }
        | Stmt::LaunchTaskWithHandle { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::SendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::TryReceivePipe { .. }
        | Stmt::StopTask { .. }
        | Stmt::CloseChannel { .. }
        | Stmt::AwaitTask { .. }
        | Stmt::Select { .. }
        | Stmt::Theorem(..)
        | Stmt::Definition(..)
        | Stmt::Axiom(..)
        | Stmt::Theory(..)
        | Stmt::Escape { .. }
        | Stmt::Require { .. }
        | Stmt::Site(_) => None,
    }
}

/// Reject any call in `file` to a user-module export that `file` doesn't have in
/// scope.
fn check_call_set(
    calls: &HashSet<Symbol>,
    file: &Path,
    scope: &ScopeInfo,
    interner: &Interner,
) -> Result<(), String> {
    // No recorded scope for this file means we can't determine what it may see —
    // stay permissive (never raise a false out-of-scope error) rather than flag
    // every export. Every contributing file is populated in `in_scope`, so this
    // only guards against an incomplete `ScopeInfo`.
    let Some(in_scope) = scope.in_scope.get(file) else {
        return Ok(());
    };
    for callee in calls {
        let name = interner.resolve(*callee);
        // Qualified `Alias::tail`: a PUBLIC tail was already rewritten to bare by
        // `resolve_qualified_names`, so a residual one is private-or-unknown. Gate
        // a private tail the file can't otherwise see (its real definition is
        // owner-mangled, so it never rewrote to bare), then skip.
        if let Some((_, tail)) = name.rsplit_once("::") {
            if !in_scope.contains(tail) {
                if let Some(err) = private_violation(tail, file, scope) {
                    return Err(err);
                }
            }
            continue;
        }
        // A name in scope — this file's own definitions or an imported module's
        // PUBLIC export — is always fine, even if the SAME spelling is `private` in
        // some OTHER module. Checking visibility FIRST is what keeps a private name
        // from poisoning a public name of the same name elsewhere.
        if in_scope.contains(name) {
            continue;
        }
        // Out of scope: a private name owned by another module is a privacy
        // violation…
        if let Some(err) = private_violation(name, file, scope) {
            return Err(err);
        }
        // …and any OTHER module's public export used without importing it is a leak.
        if scope.all_exports.contains(name) {
            let owner = scope
                .defn_file
                .get(name)
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("another module");
            let here = file
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("this file");
            return Err(format!(
                "`{name}` is used in `{here}` but not imported there — it is defined in `{owner}`. \
                 Import that module (e.g. `Import {}.`), or you cannot reach it.",
                module_alias_for(owner)
            ));
        }
    }
    Ok(())
}

/// A human module alias from a file name (`geometry.lg` → `Geometry`) for the
/// import hint.
fn module_alias_for(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(file_name);
    let mut chars = stem.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => stem.to_string(),
    }
}

/// The "private to X" error if `name` is a private name referenced from a file
/// that does NOT own it; `None` if it isn't private or the owner is referencing
/// its own private name (always allowed).
fn private_violation(name: &str, file: &Path, scope: &ScopeInfo) -> Option<String> {
    if !scope.private_names.contains(name) {
        return None;
    }
    let owner = scope.private_owner.get(name)?;
    if owner == file {
        return None;
    }
    let owner_name = owner
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("its module");
    let here = file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("this file");
    Some(format!(
        "`{name}` is private to `{owner_name}` and cannot be used in `{here}`. \
         Drop `private` from its definition to export it, or call a public \
         function of `{}` instead.",
        module_alias_for(owner_name)
    ))
}

/// The kind of a top-level definition, for collision messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    /// A `## To <name>` procedure.
    Function,
    /// A `A <Name> has/is …` type.
    Type,
}

/// A single definition site: which module (canonical path) declared a bare name,
/// and the normalized content of the definition (its identity — two defs with the
/// same normalized body are THE SAME definition).
#[derive(Debug, Clone)]
pub struct DefSite {
    /// Canonical path of the defining module file.
    pub path: PathBuf,
    /// Function or type.
    pub kind: DefKind,
    /// Whitespace-normalized definition body — the content-identity key.
    pub body: String,
}

/// Bare-name → the modules that define it. A bare name owned by more than one
/// distinct module is *ambiguous*: a bare reference to it is a hard error, and
/// the qualified form disambiguates.
#[derive(Debug, Default)]
pub struct Provenance {
    defs: std::collections::HashMap<String, Vec<DefSite>>,
}

impl Provenance {
    fn record(&mut self, name: &str, site: DefSite) {
        let entry = self.defs.entry(name.to_string()).or_default();
        // One module may textually redeclare a name; dedup by path so a single
        // module never looks like a self-collision.
        if !entry.iter().any(|s| s.path == site.path) {
            entry.push(site);
        }
    }

    /// The distinct modules that define `name` (empty if none).
    pub fn definitions(&self, name: &str) -> &[DefSite] {
        self.defs.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// True when `name` is defined by two or more modules with DIFFERENT bodies —
    /// a genuine conflict a bare reference can't resolve. Two identical
    /// definitions (same normalized body) are the same thing and merge, so they
    /// are NOT ambiguous.
    pub fn is_ambiguous(&self, name: &str) -> bool {
        let mut bodies: HashSet<&str> = HashSet::new();
        for site in self.definitions(name) {
            bodies.insert(site.body.as_str());
        }
        bodies.len() > 1
    }

    /// The first bare name defined by two or more distinct modules, if any —
    /// the collision the assembler rejects. Returns `(name, defining files)`.
    /// Deterministic: names are checked in sorted order so the message is stable.
    fn first_collision(&self) -> Option<(String, Vec<PathBuf>)> {
        let mut names: Vec<&String> = self.defs.keys().filter(|n| self.is_ambiguous(n)).collect();
        names.sort();
        names.into_iter().next().map(|name| {
            let mut paths: Vec<PathBuf> = self.defs[name].iter().map(|s| s.path.clone()).collect();
            paths.sort();
            (name.clone(), paths)
        })
    }
}

/// One contributed segment of the combined source: its `[start, end)` byte range
/// in the combined string, the ORIGINAL file it came from, and — for a byte-intact
/// segment — the offset in that file its `start` corresponds to.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Segment start in the combined source.
    pub start: usize,
    /// Segment end (exclusive) in the combined source.
    pub end: usize,
    /// The offset in the ORIGINAL file that `start` maps to. Meaningful only when
    /// `exact`; a transformed module segment leaves it 0.
    pub user_start: usize,
    /// The file that contributed this segment.
    pub file: PathBuf,
    /// True when the combined text over this segment is byte-identical to the
    /// original file's text (the entry's own leading prose + code, absent privacy
    /// mangling), so `user_start + (offset - start)` indexes the original file
    /// directly. A module segment is transformed (note-block / privacy stripping,
    /// owner-mangling) and is NOT exact.
    pub exact: bool,
}

/// A combined-source offset resolved back to its origin.
#[derive(Debug, Clone, Copy)]
pub struct Located<'a> {
    /// The originating file.
    pub file: &'a Path,
    /// Byte offset within the ORIGINAL file (valid to index it) when `exact`.
    pub user_offset: usize,
    /// The combined-source offset where the owning segment begins — lets a caller
    /// compute a best-effort segment-relative line for a non-exact segment.
    pub seg_start: usize,
    /// Whether `user_offset` indexes the original file exactly (see [`Segment::exact`]).
    pub exact: bool,
}

/// File-level attribution: for each contributed segment of the combined source,
/// its byte range, originating file, and (for byte-intact segments) the
/// offset-within-file mapping — so a diagnostic on the combined blob is
/// attributed to the user's own file and line.
#[derive(Debug, Default)]
pub struct SourceMap {
    segments: Vec<Segment>,
}

impl SourceMap {
    /// The file that contributed the text at `offset` in the combined source.
    pub fn file_for(&self, offset: usize) -> Option<&Path> {
        self.segments
            .iter()
            .find(|s| s.start <= offset && offset < s.end)
            .map(|s| s.file.as_path())
    }

    /// Resolve `offset` back to its originating file, offset-within-file, and
    /// whether that mapping is exact. `None` when `offset` is outside every
    /// segment (e.g. inside spliced trailing whitespace at the very end).
    pub fn locate(&self, offset: usize) -> Option<Located<'_>> {
        self.segments
            .iter()
            .find(|s| s.start <= offset && offset < s.end)
            .map(|s| Located {
                file: s.file.as_path(),
                user_offset: s.user_start + (offset - s.start),
                seg_start: s.start,
                exact: s.exact,
            })
    }
}

/// A module collected during the DFS, ready to be spliced.
struct Collected {
    /// Canonical path of the module file.
    path: PathBuf,
    /// The module's raw source.
    raw: String,
}

/// One import edge: the imported module and, for a SELECTIVE import, the specific
/// names it brings into scope. `None` selection = whole-module import.
struct ImportEdge {
    canon: PathBuf,
    selection: Option<Vec<SelectedName>>,
}

/// Assemble the whole project reachable from `entry_path` into one compilation
/// unit. Returns an error only for unreadable/unresolvable imports; the combined
/// source is then parsed by the normal pipeline.
pub fn assemble_project(entry_path: &Path) -> Result<AssembledProgram, String> {
    assemble_project_with_deps(entry_path, Vec::new())
}

/// [`assemble_project`], additionally linking the declared dependency roots so a
/// bare `Import <Dep>.` resolves against a `path`/`git`-local package outside the
/// project root. `dep_roots` empty is byte-for-byte identical to
/// [`assemble_project`], so a single-project build is unchanged.
pub fn assemble_project_with_deps(
    entry_path: &Path,
    dep_roots: Vec<DepRoot>,
) -> Result<AssembledProgram, String> {
    let root = entry_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader::with_dep_roots(root, dep_roots);

    let entry_raw = std::fs::read_to_string(entry_path).map_err(|e| {
        format!(
            "Failed to read entry file '{}': {}",
            entry_path.display(),
            e
        )
    })?;

    let entry_canon = canonicalize(entry_path);

    // Seed `visited` with the entry so a module that imports the entry back
    // never re-splices the entry's `## Main`.
    let mut visited: HashSet<PathBuf> = HashSet::new();
    visited.insert(entry_canon.clone());

    // Collect imported modules in post-order (dependencies first), recording
    // each module's direct import edges for per-file scoping.
    let mut ordered: Vec<Collected> = Vec::new();
    let mut imports_map: HashMap<PathBuf, Vec<ImportEdge>> = HashMap::new();
    let mut entry_imports: Vec<ImportEdge> = Vec::new();
    for dep in scan_dependencies(&entry_raw) {
        let dep_canon = {
            let module = loader.resolve(entry_path, &dep.uri)?;
            canonicalize(&module.path)
        };
        entry_imports.push(ImportEdge {
            canon: dep_canon,
            selection: dep.selection.clone(),
        });
        collect(
            entry_path,
            &dep.uri,
            &mut loader,
            &mut visited,
            &mut ordered,
            &mut imports_map,
        )?;
    }
    imports_map.insert(entry_canon.clone(), entry_imports);

    // An `Import` below a file's first `##` header imports nothing; refuse it at its own line before
    // the program assembles without the module it names.
    refuse_misplaced_imports(entry_path, &entry_raw)?;
    for m in &ordered {
        refuse_misplaced_imports(&m.path, &m.raw)?;
    }

    // Build provenance from every module (imports + entry) so a bare name
    // defined in two files is detectable regardless of where it is used.
    let mut provenance = Provenance::default();
    for m in &ordered {
        record_provenance(&mut provenance, &m.path, &m.raw);
    }
    record_provenance(&mut provenance, &entry_canon, &entry_raw);

    // Collisions are a hard error (never silent shadowing): a bare name defined
    // by two or more modules is ambiguous. The fix is to rename one, or qualify
    // the reference (`Tokens::parse` vs `Expr::parse`).
    if let Some((name, files)) = provenance.first_collision() {
        let files = files
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n  and ");
        return Err(format!(
            "`{name}` is defined in more than one module:\n  {files}\n\
             Rename one, or qualify the reference (e.g. `Module::{name}`)."
        ));
    }

    // Splice imported code into the entry, after leading prose and before the
    // entry's first `##` section — the same insertion point `apply_prelude`
    // uses, so the join parses cleanly.
    let insert_at = if entry_raw.starts_with("##") {
        0
    } else {
        entry_raw.find("\n##").map(|i| i + 1).unwrap_or(0)
    };
    let (head, orig_tail) = entry_raw.split_at(insert_at);

    let mut source = String::new();
    let mut segments: Vec<Segment> = Vec::new();

    source.push_str(head);
    if !head.is_empty() {
        // The head (the entry's leading prose) is spliced verbatim — byte-intact,
        // so a combined offset here IS the same offset in the entry file.
        segments.push(Segment {
            start: 0,
            end: source.len(),
            user_start: 0,
            file: entry_canon.clone(),
            exact: true,
        });
    }

    // Splice module code block-by-block, MERGING content-identical definitions
    // reached from more than one module (same name + byte-normalized body) so
    // they're emitted once; genuinely-different same-named defs were already
    // rejected by the collision check above.
    let mut spliced: HashMap<String, String> = HashMap::new();
    for m in &ordered {
        let code = strip_main_block(&strip_and_mangle(&m.raw, &m.path));
        if code.trim().is_empty() {
            continue;
        }
        let start = source.len();
        for (def, text) in split_blocks(&code) {
            if let Some((name, _)) = &def {
                let body = normalize_def(&text);
                if spliced.get(name).is_some_and(|prev| *prev == body) {
                    continue;
                }
                spliced.insert(name.clone(), body);
            }
            source.push_str(&text);
        }
        source.push_str("\n\n");
        // A module segment is transformed (note-block / privacy stripping,
        // owner-mangling), so its combined text is not byte-identical to the file:
        // file-accurate, line best-effort (see [`Segment::exact`]).
        segments.push(Segment {
            start,
            end: source.len(),
            user_start: 0,
            file: m.path.clone(),
            exact: false,
        });
    }

    // The entry's own code (its `## To` defs + `## Main`) also carries privacy
    // markers to strip + private functions to owner-mangle — but only the code
    // section, not the leading prose.
    let entry_map = private_mangle_map(&entry_raw, &entry_canon);
    let tail = mangle_names_in_code(&strip_privacy_and_mangle(orig_tail, &entry_map), &entry_map);
    let tail_start = source.len();
    // The tail is byte-intact iff no privacy mangling rewrote it (the common case:
    // an entry with no `private` functions). Then a combined offset maps to
    // `insert_at + (offset - tail_start)` in the entry file — an exact line.
    let tail_exact = tail == orig_tail;
    source.push_str(&tail);
    segments.push(Segment {
        start: tail_start,
        end: source.len(),
        user_start: insert_at,
        file: entry_canon.clone(),
        exact: tail_exact,
    });

    // Per-file scope: each module's own exports plus the exports of the modules
    // it directly imports. A reference to any OTHER module's export is a leak.
    let mut exports: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    let mut private_names: HashSet<String> = HashSet::new();
    // Original private name → the module that owns it (for the "private to X"
    // diagnostic). `defn_file` can't serve this — after owner-mangling it holds
    // the mangled `stem__helper`, not the bare `helper` a foreign file writes.
    let mut private_owner: HashMap<String, PathBuf> = HashMap::new();
    for m in &ordered {
        let (public, private) = module_visibility(&m.raw);
        exports.insert(m.path.clone(), public);
        for name in &private {
            private_owner
                .entry(name.clone())
                .or_insert_with(|| m.path.clone());
        }
        private_names.extend(private);
    }
    let (entry_public, entry_private) = module_visibility(&entry_raw);
    exports.insert(entry_canon.clone(), entry_public);
    for name in &entry_private {
        private_owner
            .entry(name.clone())
            .or_insert_with(|| entry_canon.clone());
    }
    private_names.extend(entry_private);

    // Per-file scope + selective narrowing. A whole-module edge brings all of a
    // dependency's public exports; a selective edge brings only its named ones.
    // A per-name alias binds the surface name to the REAL name via
    // `selective_aliases` (rewritten before every scope check / codegen), so the
    // REAL name is what lands in scope.
    let mut in_scope: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    // Per-importing-file alias maps: an alias is local to the file that declared
    // it, so it's baked only within that file's segment.
    let mut per_file_aliases: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    for (path, own) in &exports {
        let mut names = own.clone();
        if let Some(edges) = imports_map.get(path) {
            for edge in edges {
                let Some(dep_exports) = exports.get(&edge.canon) else {
                    continue;
                };
                match &edge.selection {
                    None => names.extend(dep_exports.iter().cloned()),
                    Some(sel) => {
                        for s in sel {
                            if dep_exports.contains(&s.name) {
                                names.insert(s.name.clone());
                                if let Some(alias) = &s.alias {
                                    per_file_aliases
                                        .entry(path.clone())
                                        .or_default()
                                        .insert(alias.clone(), s.name.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        in_scope.insert(path.clone(), names);
    }
    // Flattened (surface alias → real name) for the LSP getter.
    let selective_aliases: HashMap<String, String> = per_file_aliases
        .values()
        .flat_map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
    let all_exports: HashSet<String> = exports.values().flat_map(|s| s.iter().cloned()).collect();
    let defn_file: HashMap<String, PathBuf> = provenance
        .defs
        .iter()
        .filter_map(|(name, sites)| sites.first().map(|s| (name.clone(), s.path.clone())))
        .collect();

    // Bake per-name import aliases into the source so every tier resolves them —
    // each alias scoped to its importing file's segment.
    let source = bake_selective_aliases(&source, &segments, &per_file_aliases);

    Ok(AssembledProgram {
        source,
        provenance,
        source_map: SourceMap { segments },
        scope: ScopeInfo {
            in_scope,
            all_exports,
            defn_file,
            private_names,
            private_owner,
            selective_aliases,
            entry: entry_canon,
        },
    })
}

/// A module's names split by visibility: `(public, private)`. Public names are
/// what importers may reference; private names are usable only by the owning
/// module. Note blocks and prose are stripped first.
fn module_visibility(raw: &str) -> (HashSet<String>, HashSet<String>) {
    let code = strip_note_blocks(module_code(raw));
    let mut public = HashSet::new();
    let mut private = HashSet::new();
    for (name, is_private) in defined_names_with_visibility(&code) {
        if is_private {
            private.insert(name);
        } else {
            public.insert(name);
        }
    }
    // Private TYPE headers (`## A private Name` / `## An private Name`). Public
    // types are intentionally NOT added to the public set — they resolve through
    // the type registry, never the call/name scope — but a PRIVATE type must be
    // gated, so its name is recorded here.
    //
    // `## Component <Name>:` blocks: their synthesized `__render_<Name>` function
    // is a PUBLIC export of the defining module — a `Mount <Name>` or view-child
    // `<Name>` in an importing file composes it by calling that render function,
    // so an importer must have it in scope. (Components have no `private` form.)
    for line in code.lines() {
        let t = line.trim();
        let type_name = t
            .strip_prefix("## A private ")
            .or_else(|| t.strip_prefix("## An private "));
        if let Some(rest) = type_name {
            if let Some(name) = rest
                .split(|c: char| c == '(' || c == '.' || c.is_whitespace())
                .next()
            {
                if !name.is_empty() {
                    private.insert(name.to_string());
                }
            }
        }
        if let Some(render) = component_render_name(t) {
            public.insert(render);
        }
    }
    (public, private)
}

/// Drop the `private` visibility markers from definition headers AND owner-mangle
/// the header name of a private FUNCTION (`## To private helper` → `## To m__helper`
/// via `map`), so the parser sees a plain header and two modules can each own a
/// private `helper` without colliding in the flat unit. Private TYPES are not
/// mangled (only their marker dropped). Body references are mangled separately by
/// [`mangle_names_in_code`].
fn strip_privacy_and_mangle(code: &str, map: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(code.len());
    for line in code.split_inclusive('\n') {
        let indent = line.len() - line.trim_start().len();
        let (prefix, rest) = line.split_at(indent);
        let rewritten = if let Some(after) = rest.strip_prefix("## To native private ") {
            format!("## To native {}", mangle_first_word(after, map))
        } else if let Some(after) = rest.strip_prefix("## To private ") {
            format!("## To {}", mangle_first_word(after, map))
        } else if let Some(after) = rest.strip_prefix("## A private ") {
            format!("## A {after}")
        } else if let Some(after) = rest.strip_prefix("## An private ") {
            format!("## An {after}")
        } else if let Some(after) = rest.strip_prefix("A private ") {
            format!("A {after}")
        } else if let Some(after) = rest.strip_prefix("An private ") {
            format!("An {after}")
        } else {
            rest.to_string()
        };
        out.push_str(prefix);
        out.push_str(&rewritten);
    }
    out
}

/// If the first identifier word of `s` is in `map`, replace it with its mangled
/// form, preserving the rest verbatim (`"helper (n: Int):\n"` → `"m__helper (n: Int):\n"`).
fn mangle_first_word(s: &str, map: &HashMap<String, String>) -> String {
    let idx = s
        .find(|c: char| c == '(' || c.is_whitespace())
        .unwrap_or(s.len());
    let (word, rest) = s.split_at(idx);
    match map.get(word) {
        Some(mangled) => format!("{mangled}{rest}"),
        None => s.to_string(),
    }
}

/// The `## To private` FUNCTION names a module declares. Only functions are
/// owner-mangled (a private TYPE keeps its name — it is gate-only).
fn module_private_functions(raw: &str) -> HashSet<String> {
    let code = strip_note_blocks(module_code(raw));
    let mut out = HashSet::new();
    for header in code.lines().filter_map(function_header) {
        if header.private {
            out.insert(header.name.to_string());
        }
    }
    out
}

/// Owner-mangle map for a module's private functions: `helper` → `<stem>__helper`.
/// The file stem is machine-independent, so the mangled source stays byte-stable
/// across independent copies (the determinism invariant). The stem is sanitized to
/// a valid identifier first (a file like `math-utils.lg` or `2d.lg` must not mint
/// `math-utils__helper` / `2d__helper`, which the lexer/rustc would reject).
fn private_mangle_map(raw: &str, path: &Path) -> HashMap<String, String> {
    let stem = sanitize_stem(path.file_stem().and_then(|s| s.to_str()).unwrap_or("mod"));
    module_private_functions(raw)
        .into_iter()
        .map(|name| {
            let mangled = format!("{stem}__{name}");
            (name, mangled)
        })
        .collect()
}

/// A file stem reduced to a valid leading identifier fragment: non-`[A-Za-z0-9_]`
/// runs become `_`, and a leading digit (or empty stem) is prefixed with `m`.
fn sanitize_stem(stem: &str) -> String {
    let mut out: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'm');
    }
    out
}

/// Visit each `{…}` interpolation hole of `raw` EXACTLY as the parser's
/// `parse_interpolation_parts` splits them — `{{` is an escaped brace, and a hole
/// ends at its brace-DEPTH-matched `}` (so a nested `{…}` inside the expression
/// doesn't close it early). `f(content_start, content_end)` gets each hole's inner
/// byte range. Keeping this identical to the parser is the invariant that makes
/// every source-level hole rewrite land on the same span the parser later reads.
fn for_each_interpolation_hole(raw: &str, mut f: impl FnMut(usize, usize)) {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if bytes.get(i + 1) == Some(&b'{') {
                i += 2; // an escaped `{{`, not a hole
                continue;
            }
            let content_start = i + 1;
            let mut depth = 1usize;
            let mut end = content_start;
            while end < bytes.len() && depth > 0 {
                match bytes[end] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    end += 1;
                }
            }
            // `end` is the depth-matched `}` (or the raw end if unclosed).
            f(content_start, end);
            i = if end < bytes.len() { end + 1 } else { end };
        } else {
            i += 1;
        }
    }
}

/// Identifier edits INSIDE the `{…}` interpolation holes of an interpolated
/// string's source `span` (at absolute `base`) — the mangle/alias passes rewrite
/// a `{helper(n)}` call just like a bare one, while the surrounding string text
/// and a `{{` escape are left untouched. Only whole-word ASCII identifiers
/// matching `map` are rewritten (a non-ASCII byte can't start/continue one, so
/// every slice stays on a char boundary).
fn interpolation_hole_edits(
    span: &str,
    base: usize,
    map: &HashMap<String, String>,
) -> Vec<(usize, usize, String)> {
    let bytes = span.as_bytes();
    let is_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    let is_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut edits = Vec::new();
    for_each_interpolation_hole(span, |hole_start, end| {
        let mut w = hole_start;
        while w < end {
            if is_start(bytes[w]) {
                let ws = w;
                while w < end && is_cont(bytes[w]) {
                    w += 1;
                }
                if let Some(rep) = map.get(&span[ws..w]) {
                    edits.push((base + ws, base + w, rep.clone()));
                }
            } else {
                w += 1;
            }
        }
    });
    edits
}

/// Identifier edits inside the `{…}` holes of a `View:`/`Head:` raw section body — the same
/// rewrite [`interpolation_hole_edits`] performs for a legacy interpolated string, mapped back
/// through [`RawMap::offset`](logicaffeine_language::lgx::raw::RawMap::offset) since a raw body's
/// offsets are DEDENTED text, never a flat shift from the file the way a quoted string's are.
fn raw_block_hole_edits(
    raw: &logicaffeine_language::lgx::raw::RawMap,
    map: &HashMap<String, String>,
) -> Vec<(usize, usize, String)> {
    let text = raw.text.as_str();
    let bytes = text.as_bytes();
    let is_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    let is_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut edits = Vec::new();
    for_each_interpolation_hole(text, |hole_start, hole_end| {
        let mut w = hole_start;
        while w < hole_end {
            if is_start(bytes[w]) {
                let ws = w;
                while w < hole_end && is_cont(bytes[w]) {
                    w += 1;
                }
                if let Some(rep) = map.get(&text[ws..w]) {
                    edits.push((raw.offset(ws), raw.offset(w), rep.clone()));
                }
            } else {
                w += 1;
            }
        }
    });
    edits
}

/// Rewrite every identifier token named in `map` to its mangled form within
/// `code`. Plain string/char-literal content is left alone; INSIDE an interpolated
/// string's `{…}` holes, real identifiers ARE rewritten (a private call in a
/// `"{…}"` resolves like any other) — and the same is true of a `View:`/`Head:`
/// raw section's `{…}` holes, mapped back through [`RawMap`](logicaffeine_language::lgx::raw::RawMap).
/// Exact spans are replaced right-to-left so
/// earlier offsets stay valid. A no-op with an empty map.
fn mangle_names_in_code(code: &str, map: &HashMap<String, String>) -> String {
    if map.is_empty() {
        return code.to_string();
    }
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(code, &mut interner);
    let tokens = lexer.tokenize();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for token in &tokens {
        match token.kind {
            TokenType::InterpolatedString(_) => {
                if let Some(span) = code.get(token.span.start..token.span.end) {
                    edits.extend(interpolation_hole_edits(span, token.span.start, map));
                }
            }
            // A `View:`/`Head:` body is captured whole, DEDENTED (each line's leading
            // whitespace stripped), so a hole's offset inside it is not a flat shift from the
            // file the way an interpolated string's is — `RawMap` is the one place that maps
            // it back. `Style:` is CSS, never scanned: a private LOGOS name colliding with a
            // property or selector would otherwise get rewritten inside someone's stylesheet.
            TokenType::RawBlock {
                kind: RawKind::Style,
                ..
            } => {}
            TokenType::RawBlock {
                kind: RawKind::View | RawKind::Head,
                body,
                content_start,
                indent,
            } => {
                let raw = logicaffeine_language::lgx::raw::RawMap::new(
                    interner.resolve(body).to_string(),
                    indent,
                    content_start,
                );
                edits.extend(raw_block_hole_edits(&raw, map));
            }
            TokenType::StringLiteral(_) | TokenType::CharLiteral(_) => {}
            _ => {
                let text = interner.resolve(token.lexeme);
                if let Some(mangled) = map.get(text) {
                    edits.push((token.span.start, token.span.end, mangled.clone()));
                }
            }
        }
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = code.to_string();
    for (start, end, mangled) in edits {
        if start <= end
            && end <= out.len()
            && out.is_char_boundary(start)
            && out.is_char_boundary(end)
        {
            out.replace_range(start..end, &mangled);
        }
    }
    out
}

/// Strip privacy markers and owner-mangle private functions in one module's code.
fn strip_and_mangle(raw: &str, path: &Path) -> String {
    let map = private_mangle_map(raw, path);
    mangle_names_in_code(
        &strip_privacy_and_mangle(&strip_note_blocks(module_code(raw)), &map),
        &map,
    )
}

/// Resolve `uri` relative to `base_path`, then recurse into its imports
/// post-order. Dedups by canonical path (cycle- and diamond-safe), and records
/// each module's direct import edges for per-file scope resolution.
fn collect(
    base_path: &Path,
    uri: &str,
    loader: &mut Loader,
    visited: &mut HashSet<PathBuf>,
    ordered: &mut Vec<Collected>,
    imports: &mut HashMap<PathBuf, Vec<ImportEdge>>,
) -> Result<(), String> {
    let (path, raw) = {
        let module = loader.resolve(base_path, uri)?;
        (module.path.clone(), module.content.clone())
    };
    let canon = canonicalize(&path);
    if !visited.insert(canon.clone()) {
        return Ok(());
    }
    let mut my_imports = Vec::new();
    for dep in scan_dependencies(&raw) {
        let dep_canon = {
            let module = loader.resolve(&path, &dep.uri)?;
            canonicalize(&module.path)
        };
        my_imports.push(ImportEdge {
            canon: dep_canon,
            selection: dep.selection.clone(),
        });
        collect(&path, &dep.uri, loader, visited, ordered, imports)?;
    }
    imports.insert(canon.clone(), my_imports);
    ordered.push(Collected { path: canon, raw });
    Ok(())
}

/// Record every definition a module's code declares (name → site + content) so
/// collisions and content-identity can be judged.
fn record_provenance(provenance: &mut Provenance, path: &Path, raw: &str) {
    let code = strip_and_mangle(raw, path);
    for (def, text) in split_blocks(&code) {
        if let Some((name, kind)) = def {
            provenance.record(
                &name,
                DefSite {
                    path: path.to_path_buf(),
                    kind,
                    body: normalize_def(&text),
                },
            );
        }
    }
}

/// Split a module's code into `## `-headed blocks. Each entry is
/// `(Some((name, kind)) for a function/type definition else None, block text)`,
/// covering the whole code so re-joining the block texts reproduces it.
fn split_blocks(code: &str) -> Vec<(Option<(String, DefKind)>, String)> {
    let mut out = Vec::new();
    let mut cur_def: Option<(String, DefKind)> = None;
    let mut cur_text = String::new();
    let mut started = false;
    for line in code.split_inclusive('\n') {
        if line.trim_start().starts_with("## ") {
            if started {
                out.push((cur_def.take(), std::mem::take(&mut cur_text)));
            }
            started = true;
            cur_def = header_def(line);
        }
        if started {
            cur_text.push_str(line);
        }
    }
    if started {
        out.push((cur_def, cur_text));
    }
    out
}

/// Parse a `## ` header line into `(defined name, kind)`, or `None` for a
/// non-definition block (`## Main`, `## Requires`, …).
fn header_def(header: &str) -> Option<(String, DefKind)> {
    let h = header.trim();
    if h.starts_with("## To ") {
        return function_header(h).map(|header| (header.name.to_string(), DefKind::Function));
    }
    // A `## Component <Name>:` block desugars (in the parser) to a synthesized
    // FUNCTION `__render_<Name>` whose body — the View element tree, event
    // handlers, signal state, and `{…}` interpolations — carries the component's
    // real call sites. Recording that function here is what lets [`check_scopes`]
    // attribute those calls to the module the component is DEFINED in (rather than
    // silently skipping the body, or misattributing it to the entry): the render
    // function lands in `defn_file`, and its module's imports gate what it may call.
    if let Some(name) = component_render_name(h) {
        return Some((name, DefKind::Function));
    }
    let after_article = h.strip_prefix("## A ").or_else(|| h.strip_prefix("## An "));
    if let Some(rest) = after_article {
        let name = rest
            .split(|c: char| c == '(' || c == '.' || c.is_whitespace())
            .next()
            .unwrap_or("");
        return (!name.is_empty()).then(|| (name.to_string(), DefKind::Type));
    }
    None
}

/// The synthesized render-function name of a `## Component <Name>:` header, or
/// `None` for any other line. The parser desugars each component to a function
/// `__render_<Name>(__parent)` (and composes a reference like `Mount <Name>` or a
/// view child `<Name>` to a call of it), so the render function is the durable
/// identity of the component in the flat compilation unit — the one every scope /
/// provenance surface must agree on. The match mirrors the parser's own header
/// recognition (`BlockType::Component` on `## Component`, name = the next word).
fn component_render_name(header: &str) -> Option<String> {
    let rest = header.trim().strip_prefix("## Component ")?;
    let name = rest
        .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
        .next()
        .unwrap_or("");
    (!name.is_empty()).then(|| format!("__render_{name}"))
}

/// Normalize a definition block to its content identity — collapse all
/// whitespace runs so cosmetic formatting never counts as a different definition.
fn normalize_def(block: &str) -> String {
    block.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The bare names a source's code defines as FUNCTIONS (`## To <name>` /
/// `## To native <name>`), across every module spliced into it. This is the
/// self-describing set the token resolver uses to tell a qualified FUNCTION
/// reference (`Util::double`, tail is a function) from a qualified TYPE
/// reference (`Geometry::Point`, tail is a type — left namespaced).
pub(crate) fn defined_function_names(code: &str) -> HashSet<String> {
    code.lines()
        .filter_map(function_header)
        .map(|header| header.name.to_string())
        .collect()
}

/// Resolve the QUALIFIED escape hatch for FUNCTION calls: a `Alias::name` token
/// whose tail (`name`) is a known function is rewritten in place to be exactly
/// the token the bare `name` would have produced, so the call resolves to the
/// flat canonical function on every tier. The lexer glues `Alias::name` into one
/// token; the parser reads a call's callee from that token's classified kind
/// (`Noun`/`Verb`/…) as well as its `lexeme`, so we RE-LEX the bare name to get
/// the correct classification and symbol, keeping the original span.
///
/// A qualified TYPE reference (`Geometry::Point`) is left untouched: its tail is
/// a type, not a function, so it never matches. For a single-file program (no
/// `::` tokens, or none whose tail is a function) this is a no-op — the token
/// stream is byte-identical.
pub(crate) fn resolve_qualified_names(
    tokens: &mut [Token],
    interner: &mut Interner,
    functions: &HashSet<String>,
) {
    // Collect the tokens to rewrite first (their `interner.resolve` borrow must
    // end before we re-lex the bare name / re-intern the hole content, which
    // needs `&mut interner`). A bare qualified token re-lexes; an interpolated
    // string re-interns its hole-stripped content.
    let mut rewrites: Vec<(usize, String)> = Vec::new();
    let mut interp_rewrites: Vec<(usize, String)> = Vec::new();
    for (idx, token) in tokens.iter().enumerate() {
        match token.kind {
            // A `::` in a plain string/char literal is content, never a
            // namespace — but an interpolation `{Util::double(21)}` hole IS a real
            // qualified call and resolves like a bare `{double(21)}`.
            TokenType::StringLiteral(_) | TokenType::CharLiteral(_) => continue,
            TokenType::InterpolatedString(_) => {
                let raw = interner.resolve(token.lexeme);
                let edits = qualified_hole_edits(raw, functions);
                if !edits.is_empty() {
                    let mut content = raw.to_string();
                    // Right-to-left: each edit shrinks the span, so applying in
                    // reverse keeps every earlier offset valid.
                    for (start, end, tail) in edits.into_iter().rev() {
                        content.replace_range(start..end, &tail);
                    }
                    interp_rewrites.push((idx, content));
                }
                continue;
            }
            _ => {}
        }
        let text = interner.resolve(token.lexeme);
        if let Some(i) = text.rfind("::") {
            if functions.contains(&text[i + 2..]) {
                rewrites.push((idx, text[i + 2..].to_string()));
            }
        }
    }
    for (idx, bare) in rewrites {
        let span = tokens[idx].span;
        let mut lexer = Lexer::new(&bare, interner);
        let relexed = lexer.tokenize();
        if let Some(first) = relexed
            .into_iter()
            .find(|t| !matches!(t.kind, TokenType::EOF))
        {
            tokens[idx] = Token::new(first.kind, first.lexeme, span);
        }
    }
    for (idx, content) in interp_rewrites {
        let sym = interner.intern(&content);
        let span = tokens[idx].span;
        tokens[idx] = Token::new(TokenType::InterpolatedString(sym), sym, span);
    }
}

/// Qualified-call edits INSIDE the `{…}` holes of an interpolated string's raw
/// content: a `Util::double` (or `A::B::fn`) whose FINAL segment is a known
/// function is replaced by that bare tail, so a qualified call in an
/// interpolation resolves on every tier like a top-level one. Literal text, a
/// `{{` escape, and a qualified TYPE reference (tail not a function, e.g.
/// `Geometry::Point`) are left untouched. Returns byte-span edits over `raw`
/// (right-to-left when applied, since spans only shrink).
fn qualified_hole_edits(raw: &str, functions: &HashSet<String>) -> Vec<(usize, usize, String)> {
    let bytes = raw.as_bytes();
    let is_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    let is_seg = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut edits = Vec::new();
    for_each_interpolation_hole(raw, |hole_start, end| {
        let mut w = hole_start;
        while w < end {
            if is_start(bytes[w]) {
                let word_start = w;
                let mut tail_start = w;
                let mut saw_colon = false;
                loop {
                    while w < end && is_seg(bytes[w]) {
                        w += 1;
                    }
                    // A `::` joins segments only when another identifier follows.
                    if w + 2 < end
                        && bytes[w] == b':'
                        && bytes[w + 1] == b':'
                        && is_start(bytes[w + 2])
                    {
                        w += 2;
                        tail_start = w;
                        saw_colon = true;
                    } else {
                        break;
                    }
                }
                if saw_colon && functions.contains(&raw[tail_start..w]) {
                    edits.push((word_start, w, raw[tail_start..w].to_string()));
                }
            } else {
                w += 1;
            }
        }
    });
    edits
}

/// Rewrite each per-name import alias to its real exported name DIRECTLY IN THE
/// ASSEMBLED SOURCE, so that a `dbl` reference (`Import double as dbl from Util.`)
/// resolves to `double` on every tier. An alias is LOCAL to the importing file, so
/// a token is rewritten ONLY when it sits inside that file's own segment (per
/// `segments`) — an unrelated `dbl` in another module is never disturbed. We lex,
/// map each token to its owning file, and replace the exact spans right-to-left. A
/// no-op with no aliases (byte-identical source).
fn bake_selective_aliases(
    source: &str,
    segments: &[Segment],
    per_file: &HashMap<PathBuf, HashMap<String, String>>,
) -> String {
    if per_file.is_empty() {
        return source.to_string();
    }
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for token in &tokens {
        // A `dbl` inside a plain string/char literal is content; inside an
        // interpolation `{…}` hole it is a real reference and IS rewritten.
        if matches!(
            token.kind,
            TokenType::StringLiteral(_) | TokenType::CharLiteral(_)
        ) {
            continue;
        }
        let owner = segments
            .iter()
            .find(|s| s.start <= token.span.start && token.span.start < s.end)
            .map(|s| &s.file);
        let Some(map) = owner.and_then(|f| per_file.get(f)) else {
            continue;
        };
        if matches!(token.kind, TokenType::InterpolatedString(_)) {
            if let Some(span) = source.get(token.span.start..token.span.end) {
                edits.extend(interpolation_hole_edits(span, token.span.start, map));
            }
            continue;
        }
        let text = interner.resolve(token.lexeme);
        if let Some(real) = map.get(text) {
            edits.push((token.span.start, token.span.end, real.clone()));
        }
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = source.to_string();
    for (start, end, real) in edits {
        if start <= end
            && end <= out.len()
            && out.is_char_boundary(start)
            && out.is_char_boundary(end)
        {
            out.replace_range(start..end, &real);
        }
    }
    out
}

/// A module's code with any `## Main` block removed — from the `## Main` header
/// through the line before the next `##` header (or EOF). Only the entry's
/// `## Main` is kept; an imported module contributes definitions only.
pub(crate) fn strip_main_block(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_main = false;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("## ") {
            in_main = trimmed.trim_end() == "## Main";
            if in_main {
                continue;
            }
        }
        if in_main {
            continue;
        }
        out.push_str(line);
    }
    out
}

/// Every `Import` sentence in `raw` that stands below the file's first `##` header, refused at its own
/// `path:line:col`: `scan_dependencies` reads imports only from the prose above that header, so such a
/// sentence imports nothing and the program would otherwise run on without the module it names.
fn refuse_misplaced_imports(path: &Path, raw: &str) -> Result<(), String> {
    let misplaced = misplaced_imports(raw);
    let Some((header_line, header)) = first_section_header(raw).filter(|_| !misplaced.is_empty())
    else {
        return Ok(());
    };
    let mut seen = HashSet::new();
    let refusals: Vec<String> = misplaced
        .iter()
        .filter(|dep| seen.insert(dep.start))
        .map(|dep| {
            let line = raw[..dep.start].matches('\n').count() + 1;
            let col = dep.start - raw[..dep.start].rfind('\n').map_or(0, |i| i + 1) + 1;
            let sentence = raw[dep.start..dep.end].trim();
            format!(
                "`{sentence}` at {}:{line}:{col} is below the `{header}` header on line {header_line}: an Import \
                 imports only from the prose above a file's first `##` section, so this one imports nothing. \
                 Move it above line {header_line}.",
                path.display()
            )
        })
        .collect();
    Err(refusals.join("\n"))
}

/// Canonicalize a path for dedup; fall back to the path itself if the file
/// cannot be canonicalized (it was just read, so this rarely fails).
fn canonicalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/assemble/tests.rs"]
mod tests;
