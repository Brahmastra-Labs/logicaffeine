//! What a project IS — decided once, read by every `largo` command.
//!
//! A project is one of four shapes, and the shape follows from two facts: does it have a site
//! (`pages/` or `static/`), and does its entry program `Serve`?
//!
//! | shape                       | `pages/` or `static/` | the entry `Serve`s |
//! |-----------------------------|-----------------------|--------------------|
//! | [`ProjectShape::Site`]      | yes                   | no, or no entry    |
//! | [`ProjectShape::FullStack`] | yes                   | yes                |
//! | [`ProjectShape::Backend`]   | no                    | yes                |
//! | [`ProjectShape::Program`]   | no                    | no                 |
//!
//! `Serve` is an AST fact over the entry's assembled closure — a `Serve` in an imported module
//! counts, one named in a string or a comment does not. What each command does with a shape is a
//! table on the shape itself ([`ProjectShape::serve_plan`], [`ProjectShape::build_plan`]), so
//! `largo serve` and `largo build` cannot disagree about what a project is, and `check`, `test`
//! and `fmt` read their program set from the same [`Project`].
//!
//! A site needs no entry program: a `## Page` mounts itself. `Largo.toml` names an entry only
//! when the project has one — an `entry` the manifest DECLARES must exist, while the conventional
//! default (`src/main.lg`) may be absent from a site.
//!
//! [`Project::shape`] is the STRICT decision (a parse failure is an error); [`Project::shape_or_best_effort`]
//! is the same decision made from what can still be read when the entry does not parse — the file
//! layout and a lexical `Serve` scan — for `largo serve --watch`, which must bind a port and show
//! the located error rather than refuse to start over an author's typo.

use std::path::{Path, PathBuf};

use logicaffeine_language::ast::stmt::Stmt;

use crate::project::manifest::Manifest;
use crate::ui::CliError;

/// What a project is. Decided by [`Project::shape`]; every command reads it rather than re-deriving
/// a piece of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectShape {
    /// Pages and assets and no `Serve` backend: a website that is files.
    Site,
    /// A `Serve` program and nothing else to serve.
    Backend,
    /// A `Serve` program beside `pages/` or `static/`.
    FullStack,
    /// Neither — a program.
    Program,
}

/// What `largo serve` stands up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServePlan {
    /// `pages/` and `static/` from memory, rebuilt and reloaded on every save. With `proxy`, a
    /// request nothing local answers goes to the entry's `Serve` program running as a child;
    /// without it, it is answered `404`, exactly as a static host answers it.
    Pages {
        /// Whether a backend child answers what the site does not.
        proxy: bool,
    },
    /// The entry's `Serve` binds the port itself, on the interpreter.
    Backend,
    /// The entry program itself: its WebAssembly bundle when it builds to one, else the
    /// interpreter runs it.
    Program,
}

/// What a plain `largo build` produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildPlan {
    /// `dist/` — the pre-rendered pages, their modules and `static/`, for any static host. No
    /// cargo, no rustc: there is no program to compile.
    StaticExport,
    /// One native binary. With `embeds_site`, the pages and `static/` ride inside it and are
    /// answered before the program's own routes.
    Binary {
        /// Whether the site is baked into the binary.
        embeds_site: bool,
    },
}

impl ProjectShape {
    /// Every shape, for the censuses that must cover them all.
    pub const ALL: [ProjectShape; 4] = [
        ProjectShape::Site,
        ProjectShape::Backend,
        ProjectShape::FullStack,
        ProjectShape::Program,
    ];

    /// The shape two facts decide.
    pub fn from_facts(has_site: bool, entry_serves: bool) -> ProjectShape {
        match (has_site, entry_serves) {
            (true, false) => ProjectShape::Site,
            (true, true) => ProjectShape::FullStack,
            (false, true) => ProjectShape::Backend,
            (false, false) => ProjectShape::Program,
        }
    }

    /// What `largo serve` does with this shape.
    pub fn serve_plan(self) -> ServePlan {
        match self {
            ProjectShape::Site => ServePlan::Pages { proxy: false },
            ProjectShape::FullStack => ServePlan::Pages { proxy: true },
            ProjectShape::Backend => ServePlan::Backend,
            ProjectShape::Program => ServePlan::Program,
        }
    }

    /// What a plain `largo build` does with this shape.
    pub fn build_plan(self) -> BuildPlan {
        match self {
            ProjectShape::Site => BuildPlan::StaticExport,
            ProjectShape::FullStack => BuildPlan::Binary { embeds_site: true },
            ProjectShape::Backend | ProjectShape::Program => {
                BuildPlan::Binary { embeds_site: false }
            }
        }
    }

    /// Whether the project has a site — pages and assets that `check`, `test`, `fmt`, `serve` and
    /// `build` all read.
    pub fn has_site(self) -> bool {
        matches!(self, ProjectShape::Site | ProjectShape::FullStack)
    }

    /// The shape in the words a message uses.
    pub fn describe(self) -> &'static str {
        match self {
            ProjectShape::Site => "a site (pages and assets, no `Serve` backend)",
            ProjectShape::FullStack => {
                "a full-stack site (pages and assets beside a `Serve` backend)"
            }
            ProjectShape::Backend => "a backend (a `Serve` program)",
            ProjectShape::Program => "a program",
        }
    }
}

/// A project on disk: its manifest, its entry program if it has one, and its directories.
///
/// Loading reads no source. [`Project::shape`] parses the entry; commands that only need the
/// program set (`check`, `test`, `fmt`) never pay for — or fail on — that parse.
#[derive(Debug, Clone)]
pub struct Project {
    /// The directory holding `Largo.toml`.
    pub root: PathBuf,
    /// The parsed manifest.
    pub manifest: Manifest,
    /// The entry program, or `None` for a site that has none.
    pub entry: Option<PathBuf>,
}

impl Project {
    /// Load the project rooted at `root`.
    pub fn load(root: &Path) -> Result<Project, CliError> {
        let manifest = Manifest::load(root).map_err(|e| CliError::new(e.to_string()))?;
        let entry = entry_file(root, &manifest)?;
        Ok(Project {
            root: root.to_path_buf(),
            manifest,
            entry,
        })
    }

    /// `pages/` — one program per `.lg` file, each served at its own URL.
    pub fn pages_dir(&self) -> PathBuf {
        self.root.join("pages")
    }

    /// `static/` — files served verbatim at root-relative paths.
    pub fn static_dir(&self) -> PathBuf {
        self.root.join("static")
    }

    /// Whether the project has a site: a `pages/` or a `static/` directory.
    pub fn has_site(&self) -> bool {
        self.pages_dir().is_dir() || self.static_dir().is_dir()
    }

    /// The shape: [`ProjectShape::from_facts`] over [`Project::has_site`] and whether the entry's
    /// assembled program contains a `Serve` statement.
    ///
    /// An entry that does not assemble or does not parse is an error here rather than a guess: a
    /// full-stack site whose backend has a syntax error must not quietly become a site without
    /// one. The error is the same rendered diagnostic `largo check` prints.
    pub fn shape(&self) -> Result<ProjectShape, CliError> {
        let serves = match &self.entry {
            None => false,
            Some(entry) => {
                let dep_roots =
                    crate::project::deps::resolve_dep_roots(&self.root, &self.manifest)?;
                let closure = crate::closure::assemble_closure_with_deps(entry, dep_roots.clone())
                    .map_err(|e| CliError::new(format!("could not assemble project: {e}")))?;
                match program_serves(&closure.source) {
                    Some(serves) => serves,
                    None => return Err(crate::diagnostics::entry_failure(entry, dep_roots)),
                }
            }
        };
        Ok(ProjectShape::from_facts(self.has_site(), serves))
    }

    /// The entry program, or the error that says this project is a site and has none — for the
    /// commands (`run`, `emit`, `doc`, …) that are about a program.
    pub fn require_entry(&self) -> Result<&Path, CliError> {
        self.entry
            .as_deref()
            .ok_or_else(|| site_without_entry(&self.root))
    }

    /// The shape, but NEVER refusing over a parse error: [`Project::shape`]'s exact answer when
    /// the entry parses, and a shape decided from what can still be read — `has_site()` (a file-
    /// system fact, unaffected by any parse) and a LEXICAL (not grammatical) scan for a `Serve`
    /// statement — when it does not.
    ///
    /// A watch-mode dev server exists to survive the author's mistakes: starting `largo serve
    /// --watch` on a page mid-typo, or a save that breaks it, must still bind a port and show the
    /// located error, never refuse to start. `Project::shape` remains the STRICT form for every
    /// other command (`check`, `build`, `test`, `fmt`, `doctor`) — they want "and it currently
    /// parses" folded into the answer, and their own pipelines report the parse error at the
    /// right place regardless (`plan_web_bundle`'s own comment: "the compile step below reports
    /// that at its line, so it is not reported twice here").
    pub fn shape_or_best_effort(&self) -> ProjectShape {
        let serves = match &self.entry {
            None => false,
            Some(entry) => {
                let source = crate::project::deps::resolve_dep_roots(&self.root, &self.manifest)
                    .ok()
                    .and_then(|dep_roots| {
                        crate::closure::assemble_closure_with_deps(entry, dep_roots).ok()
                    })
                    .map(|closure| closure.source)
                    .or_else(|| std::fs::read_to_string(entry).ok());
                match &source {
                    Some(src) => {
                        program_serves(src).unwrap_or_else(|| source_lexes_a_serve_statement(src))
                    }
                    None => false,
                }
            }
        };
        ProjectShape::from_facts(self.has_site(), serves)
    }
}

/// Whether `source` LEXES a bare `Serve` statement-start token anywhere — tokenizing almost never
/// fails where PARSING a full grammar can (a lexer classifies words; it does not validate
/// sentences), so a typo elsewhere in the file — even one that stops the parser before it ever
/// reaches a `Serve` nested in an `If` or a function — does not blind this to a `Serve` the author
/// clearly wrote. Two things the parser would never read as a `Serve` statement, and neither can
/// this: anything the lexer folded into a string or an interpolation (`Show "Serve on port 8080
/// with handle."` is ONE token there, never separately lexed as the bare word `Serve`, exactly
/// like [`program_serves`]'s "an AST fact, not a substring guess" — this is that same fact, read
/// one level down, at the token the parser could not reach); and anything inside a `## Note` —
/// documentation, which the parser never turns into statements at all, so a `Serve` mentioned in
/// its prose ("`Serve` is a word this program never calls as a statement") must not read as one.
fn source_lexes_a_serve_statement(source: &str) -> bool {
    use logicaffeine_language::token::{BlockType, TokenType};
    let mut interner = logicaffeine_base::Interner::new();
    let tokens = logicaffeine_language::lexer::Lexer::new(source, &mut interner).tokenize();
    let mut in_note = false;
    for t in &tokens {
        if let TokenType::BlockHeader { block_type } = t.kind {
            in_note = matches!(block_type, BlockType::Note);
            continue;
        }
        if in_note {
            continue;
        }
        if !matches!(
            t.kind,
            TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)
        ) && interner.resolve(t.lexeme) == "Serve"
        {
            return true;
        }
    }
    false
}

/// The error a command that needs a program gets from a site that has none.
pub(crate) fn site_without_entry(root: &Path) -> CliError {
    CliError::with_hint(
        format!(
            "{} is a site: its pages are in pages/ and it has no entry program",
            root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "this project".into())
        ),
        "`largo serve` shows it, `largo build` exports it to dist/, and `largo test` runs its pages' tests",
    )
}

/// The project's entry file: the manifest's `entry`, else its `.md` sibling.
///
/// `Ok(None)` only for a site whose manifest declares no `entry` — the conventional default is
/// allowed to be absent there. A DECLARED entry that is missing is an error for every shape, or a
/// misspelled backend would silently turn a full-stack site into a site with no API.
///
/// The entry is confined to the project: absolute paths and `..` escapes are rejected, so a
/// distributed manifest can never point commands at arbitrary files on the machine.
pub(crate) fn entry_file(root: &Path, manifest: &Manifest) -> Result<Option<PathBuf>, CliError> {
    let declared = Path::new(&manifest.package.entry);
    if declared.is_absolute()
        || declared
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(CliError::with_hint(
            format!(
                "the manifest entry {:?} escapes the project directory",
                manifest.package.entry
            ),
            "`entry` must be a relative path inside the project (like `src/main.lg`)",
        ));
    }
    let entry = root.join(declared);
    if entry.exists() {
        return Ok(Some(entry));
    }
    let md = entry.with_extension("md");
    if md.exists() {
        return Ok(Some(md));
    }
    let has_site = root.join("pages").is_dir() || root.join("static").is_dir();
    if has_site && !manifest_declares_entry(root) {
        return Ok(None);
    }
    Err(CliError::with_hint(
        format!(
            "entry point not found: {} (also tried {})",
            entry.display(),
            md.display()
        ),
        "check the `entry` field in Largo.toml",
    ))
}

/// Whether `Largo.toml` names an `entry` itself, as opposed to leaving the default in force.
fn manifest_declares_entry(root: &Path) -> bool {
    std::fs::read_to_string(root.join("Largo.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| {
            table
                .get("package")
                .and_then(|p| p.get("entry"))
                .map(|_| ())
        })
        .is_some()
}

/// Whether an assembled program contains a `Serve` statement anywhere — top level, nested in
/// control flow, or inside a function. `None` when it does not parse.
pub(crate) fn program_serves(source: &str) -> Option<bool> {
    logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, _interner| match parsed {
        Ok((stmts, _types, _policies)) => Some(stmts_contain_serve(stmts)),
        Err(_) => None,
    })
}

/// Recursive `Stmt::Serve` search over every block-bearing statement.
fn stmts_contain_serve(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_contains_serve)
}

fn stmt_contains_serve(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Serve { .. } => true,
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            stmts_contain_serve(then_block) || else_block.map_or(false, |b| stmts_contain_serve(b))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            stmts_contain_serve(body)
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => stmts_contain_serve(tasks),
        Stmt::FunctionDef { body, .. } => stmts_contain_serve(body),
        Stmt::Inspect { arms, .. } => arms.iter().any(|arm| stmts_contain_serve(arm.body)),
        _ => false,
    }
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/project/shape/tests.rs"]
mod tests;
