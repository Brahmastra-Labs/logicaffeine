//! `largo check` — parse and type-check every program in the project without producing a binary.
//!
//! The program set is the project's: its entry, when it has one, and every page in `pages/`, each
//! checked on the tier it ships on. Every finding is placed in the author's own file
//! ([`crate::diagnostics::Placed`]) and reported identically on stderr and in `--json`.

use std::path::Path;

use logicaffeine_compile::loader::DepRoot;
use logicaffeine_language::diagnostic::Diagnostic;

use crate::commands::require_project_root;
use crate::compile::compile_project_with_deps;
use crate::diagnostics::{self, Placed};
use crate::project::shape::Project;
use crate::ui::{self, CliError};

/// What a check found, in the order it found it.
#[derive(Default)]
pub(crate) struct Findings {
    /// Every finding as a `logos-diagnostic/1` object, for `--json`.
    pub diagnostics: Vec<Diagnostic>,
    /// The rendered errors, for the terminal.
    pub errors: Vec<String>,
}

impl Findings {
    fn error(&mut self, rendered: String, diagnostic: Diagnostic) {
        self.errors.push(rendered);
        self.diagnostics.push(diagnostic);
    }

    /// Record a warning, printing it now unless the output is the `--json` envelope.
    fn warning(&mut self, rendered: String, diagnostic: Diagnostic, json: bool) {
        if !json {
            ui::warning(&rendered);
        }
        self.diagnostics.push(diagnostic);
    }

    /// Every LGX finding in a program's closure, placed in the file it is about.
    fn lgx(&mut self, closure: &crate::closure::Closure, file: &Path, json: bool) {
        for f in logicaffeine_compile::ui_bridge::lgx_findings_of(&closure.source) {
            let at = Placed::locate(f.span, &closure.source, &closure.source_map, file);
            let rendered = diagnostics::render_finding(&f, &at);
            let diagnostic = diagnostics::finding_diagnostic(&f, &at);
            match f.severity {
                logicaffeine_language::lgx::check::Severity::Error => {
                    self.error(rendered, diagnostic)
                }
                logicaffeine_language::lgx::check::Severity::Warning => {
                    self.warning(rendered, diagnostic, json)
                }
            }
        }
    }

    /// Every value-semantics copy warning in a program's closure, placed in the file it is about —
    /// the same treatment `lgx` gives its own findings. Every kind this lint has today is a
    /// warning (never blocks the check), but the match stays exhaustive so a future error kind
    /// cannot silently fall through as one.
    fn copy_lint(&mut self, closure: &crate::closure::Closure, file: &Path, json: bool) {
        for f in logicaffeine_compile::ui_bridge::value_copy_findings_of(&closure.source) {
            let at = Placed::locate(f.span, &closure.source, &closure.source_map, file);
            let rendered = diagnostics::render_copy_finding(&f, &at);
            let diagnostic = diagnostics::copy_finding_diagnostic(&f, &at);
            match f.severity {
                logicaffeine_language::lint::Severity::Warning => {
                    self.warning(rendered, diagnostic, json)
                }
            }
        }
    }

    /// Every refusal `largo build` would compile into the entry's binary.
    ///
    /// The check compiles the entry to Rust and discards it, and that compile hands back the Rust
    /// of a program it refused — the refusal is a `compile_error!` inside it — so a backend could
    /// check clean and then not build. A program that touches the DOM is a browser program, which
    /// ships as WebAssembly (`largo serve`), and is not read against the native tier.
    fn deploy_refusals(&mut self, rust: &str, closure: &crate::closure::Closure, file: &Path) {
        use logicaffeine_compile::aot_unsupported::{first_reachable, Offload};
        let browser = logicaffeine_compile::compile::compile_to_program(&closure.source)
            .is_ok_and(|program| first_reachable(&program) == Some(Offload::Dom));
        if browser {
            return;
        }
        let mut seen: Vec<String> = Vec::new();
        for refusal in rust.lines().filter_map(compile_error_message) {
            if seen.contains(&refusal) {
                continue;
            }
            let message = format!("`largo build` refuses this program: {refusal}");
            self.error(
                format!("{}: {message}", file.display()),
                diagnostics::file_failure(file, "deploy-refusal", &message),
            );
            seen.push(refusal);
        }
    }

    /// The check's verdict: `Ok` with every diagnostic when nothing is an error; else every error
    /// but the last printed and the last returned, so each appears once under `error:`.
    fn verdict(mut self, json: bool) -> Result<Vec<Diagnostic>, Box<dyn std::error::Error>> {
        let Some(last) = self.errors.pop() else {
            return Ok(self.diagnostics);
        };
        if json {
            println!("{}", diagnostics::check_envelope(false, &self.diagnostics));
        }
        for earlier in &self.errors {
            ui::error(earlier);
        }
        Err(CliError::new(last).into())
    }
}

/// The message of the `compile_error!("…")` on `line`, if it carries one.
fn compile_error_message(line: &str) -> Option<String> {
    const OPEN: &str = "compile_error!(\"";
    let rest = &line[line.find(OPEN)? + OPEN.len()..];
    let mut message = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(message),
            '\\' => message.push(chars.next()?),
            other => message.push(other),
        }
    }
    None
}

/// Check every page in `project`'s `pages/` on the tier a page actually SHIPS on
/// (`compile_to_wasm`, what the dev server and the release bundler both use), through the same
/// closure assembly, so an `Import` resolves here exactly as it does there.
pub(crate) fn check_pages(project: &Project, dep_roots: &[DepRoot], json: bool) -> Findings {
    let mut found = Findings::default();
    let pages = match crate::commands::web_bundle::page_files(&project.pages_dir()) {
        Ok(pages) => pages,
        Err(e) => {
            let message = e.to_string();
            found.error(
                message.clone(),
                diagnostics::file_failure(&project.pages_dir(), "unreadable-pages", &message),
            );
            return found;
        }
    };
    for (stem, path) in pages {
        let closure = match crate::closure::assemble_closure_with_deps(&path, dep_roots.to_vec()) {
            Ok(closure) => closure,
            Err(message) => {
                found.error(
                    format!("pages/{stem}.lg: {message}"),
                    diagnostics::file_failure(&path, "assembly-failure", &message),
                );
                continue;
            }
        };
        found.lgx(&closure, &path, json);
        found.copy_lint(&closure, &path, json);
        if let Err(pe) = logicaffeine_compile::compile::compile_to_wasm(&closure.source) {
            let at = Placed::locate(pe.span, &closure.source, &closure.source_map, &path);
            found.error(
                diagnostics::render_parse_error(&pe, &at),
                diagnostics::parse_error_diagnostic(&pe, &at),
            );
        }
    }
    found
}

/// Handle `largo check`: compile every program to its tier in memory and discard the output.
/// With `--deep`, additionally run `cargo check` over the entry's generated code and translate
/// rustc's findings back to LOGOS — the IDE flycheck's exact pass, reproducible from the command
/// line. With `--json`, print one `logos-check/1` envelope on stdout (the prose keeps flowing to
/// stderr); exit codes are unchanged.
pub(crate) fn cmd_check(deep: bool, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let project = Project::load(&require_project_root()?)?;
    // Link the declared local (`path`/`git`) dependencies: their roots are admitted by the loader
    // so a bare `Import <Dep>.` resolves outside the project, and an unlinkable/floating
    // declaration fails loudly here.
    let dep_roots = crate::project::deps::resolve_dep_roots(&project.root, &project.manifest)?;
    let mut found = Findings::default();

    if let Some(entry) = &project.entry {
        match compile_project_with_deps(entry, dep_roots.clone()) {
            Ok(output) => {
                if let Ok(closure) =
                    crate::closure::assemble_closure_with_deps(entry, dep_roots.clone())
                {
                    found.lgx(&closure, entry, json);
                    found.copy_lint(&closure, entry, json);
                    found.deploy_refusals(&output.rust_code, &closure, entry);
                }
            }
            Err(e) => {
                let diagnostic = match &e {
                    logicaffeine_compile::compile::CompileError::Parse(pe) => {
                        diagnostics::entry_parse_diagnostic(pe, entry, &dep_roots)
                    }
                    other => {
                        diagnostics::file_failure(entry, "compile-failure", &other.to_string())
                    }
                };
                found.error(
                    diagnostics::compile_error(&e, entry, &dep_roots).message,
                    diagnostic,
                );
            }
        }
    }
    let pages = check_pages(&project, &dep_roots, json);
    found.errors.extend(pages.errors);
    found.diagnostics.extend(pages.diagnostics);

    let mut diagnostics = found.verdict(json)?;
    let Some(entry_path) = project.entry.clone() else {
        // A site with no entry program: its pages are the whole of it.
        if deep {
            return Err(project.require_entry().unwrap_err().into());
        }
        if json {
            println!("{}", diagnostics::check_envelope(true, &diagnostics));
        } else {
            ui::info("Check passed");
        }
        return Ok(());
    };

    if deep {
        // Deep-check is a secondary rustc-agreement pass over the SAME assembled closure every
        // other command compiles — so an imported module's symbols are in scope and a call to one
        // is never a phantom "cannot find function" finding. Each rustc finding's byte offset (in
        // the combined source) is mapped back through the closure's `SourceMap` to the user's own
        // file+line: exact for the entry's own code, file-accurate / line-best-effort for a
        // transformed module.
        let closure = crate::closure::assemble_closure_with_deps(&entry_path, dep_roots.clone())
            .map_err(|e| CliError::new(format!("could not assemble project: {e}")))?;
        // Key the flycheck cache on the PROJECT PATH, not just the package name: two projects both
        // named `hello` (the common default) must never share — or concurrently race on — one
        // generated workspace.
        let root_hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            project.root.hash(&mut h);
            h.finish()
        };
        let cache_dir = std::env::temp_dir().join(format!(
            "logicaffeine-flycheck-{}-{root_hash:016x}",
            project.manifest.package.name
        ));
        let findings = logicaffeine_compile::compile::rustc_check(&closure.source, &cache_dir)
            .map_err(|e| {
                CliError::new(format!(
                    "deep check could not run: {}",
                    diagnostics::compile_error(&e, &entry_path, &dep_roots).message
                ))
            })?;
        if !findings.is_empty() {
            if json {
                diagnostics.extend(
                    findings
                        .iter()
                        .map(|finding| deep_finding_diagnostic(finding, &closure, &entry_path)),
                );
                println!("{}", diagnostics::check_envelope(false, &diagnostics));
            }
            for finding in &findings {
                let mut message = format!("{}\n{}", finding.title, finding.explanation);
                if let Some(suggestion) = &finding.suggestion {
                    message.push('\n');
                    message.push_str(suggestion);
                }
                if let Some(span) = finding.logos_span {
                    let (file, line) = deep_finding_location(&closure, &entry_path, span.start);
                    message.push_str(&format!("\n  --> {}:{line}", file.display()));
                }
                eprintln!("{message}\n");
            }
            return Err(format!("{} deep-check finding(s)", findings.len()).into());
        }
        if json {
            println!("{}", diagnostics::check_envelope(true, &diagnostics));
        } else {
            ui::info("Deep check passed (rustc agrees)");
        }
        return Ok(());
    }

    // Compilation succeeded — run import-hygiene lints (unused / duplicate / self-import).
    // Warnings keep `ok: true` and exit 0; only errors fail.
    let lints = logicaffeine_compile::analysis::lint_imports_with_deps(&entry_path, dep_roots)
        .unwrap_or_default();
    let has_error = lints
        .iter()
        .any(|l| l.severity == logicaffeine_compile::analysis::LintSeverity::Error);

    if json {
        let source = std::fs::read_to_string(&entry_path).unwrap_or_default();
        diagnostics.extend(
            lints
                .iter()
                .map(|l| import_hygiene_diagnostic(l, &source, &entry_path)),
        );
        println!("{}", diagnostics::check_envelope(!has_error, &diagnostics));
    } else {
        for l in &lints {
            let word = match l.severity {
                logicaffeine_compile::analysis::LintSeverity::Error => "error",
                logicaffeine_compile::analysis::LintSeverity::Warning => "warning",
            };
            eprintln!("{word}: {}", l.message);
        }
        if !has_error {
            ui::info("Check passed");
        }
    }

    if has_error {
        return Err(format!(
            "{} import error(s)",
            lints
                .iter()
                .filter(|l| l.severity == logicaffeine_compile::analysis::LintSeverity::Error)
                .count()
        )
        .into());
    }
    Ok(())
}

/// An import-hygiene lint as a `logos-diagnostic/1` object.
fn import_hygiene_diagnostic(
    lint: &logicaffeine_compile::analysis::ImportLint,
    source: &str,
    entry_path: &std::path::Path,
) -> logicaffeine_language::diagnostic::Diagnostic {
    use logicaffeine_compile::analysis::{ImportLintKind, LintSeverity};
    use logicaffeine_language::diagnostic::{line_col, DiagSpan, Diagnostic};

    let start = lint.span.0.min(source.len());
    let end = lint.span.1.min(source.len());
    let (start_line, start_col) = line_col(source, start);
    let (end_line, end_col) = line_col(source, end);
    let code = match lint.kind {
        ImportLintKind::Unused => "unused-import",
        ImportLintKind::Duplicate => "duplicate-import",
        ImportLintKind::SelfImport => "self-import",
    };
    let severity = match lint.severity {
        LintSeverity::Error => "error",
        LintSeverity::Warning => "warning",
    };
    Diagnostic {
        code: code.to_string(),
        severity: severity.to_string(),
        file: Some(entry_path.display().to_string()),
        span: DiagSpan {
            start,
            end,
            start_line,
            start_col,
            end_line,
            end_col,
        },
        message: lint.message.clone(),
        explanation: String::new(),
        suggestion: None,
        quickfix: None,
        docs: None,
        related: Vec::new(),
    }
}

/// A rustc deep-check finding as a `logos-diagnostic/1` object. Deep
/// findings carry their own prose, not a `ParseErrorKind`, so the code is
/// the fixed `deep-check` class. The finding's LOGOS span is a COMBINED-source
/// offset; the `file` + `start_line` are mapped back through the closure's
/// `SourceMap` to the originating file, while columns and the span's line-height
/// are read from the combined source (byte-intact over the entry's own code, so
/// a column is preserved; best-effort inside a transformed module).
fn deep_finding_diagnostic(
    finding: &logicaffeine_compile::diagnostic::LogosError,
    closure: &crate::closure::Closure,
    entry_path: &std::path::Path,
) -> logicaffeine_language::diagnostic::Diagnostic {
    use logicaffeine_language::diagnostic::{line_col, DiagSpan, Diagnostic};

    let src = &closure.source;
    let (start, end) = finding
        .logos_span
        .map(|s| (s.start.min(src.len()), s.end.min(src.len())))
        .unwrap_or((0, 0));
    let (file, start_line) = deep_finding_location(closure, entry_path, start);
    let (_, start_col) = line_col(src, start);
    // Preserve the span's line-height across the mapping (combined and original
    // agree on newline count within a byte-intact segment).
    let comb_start = line_col(src, start).0;
    let comb_end = line_col(src, end).0;
    let end_line = start_line + comb_end.saturating_sub(comb_start);
    let (_, end_col) = line_col(src, end);
    Diagnostic {
        code: "deep-check".to_string(),
        severity: "error".to_string(),
        file: Some(file.display().to_string()),
        span: DiagSpan {
            start,
            end,
            start_line,
            start_col,
            end_line,
            end_col,
        },
        message: finding.title.clone(),
        explanation: finding.explanation.clone(),
        suggestion: finding.suggestion.clone(),
        quickfix: None,
        docs: None,
        related: Vec::new(),
    }
}

/// Map a combined-source byte `offset` (a deep-check finding's LOGOS span start)
/// back to the file + 1-based line to show the user. Exact for the entry's own
/// code — the `SourceMap` indexes the original file directly; for a transformed
/// imported module the file is right and the line is a segment-relative
/// best-effort. Falls back to the entry file + combined-source line when the
/// offset lands outside every mapped segment.
fn deep_finding_location(
    closure: &crate::closure::Closure,
    entry_path: &std::path::Path,
    offset: usize,
) -> (std::path::PathBuf, usize) {
    match closure.source_map.locate(offset) {
        Some(loc) if loc.exact => {
            let text = std::fs::read_to_string(loc.file).unwrap_or_default();
            (loc.file.to_path_buf(), line_at(&text, loc.user_offset))
        }
        Some(loc) => {
            // A transformed module: the file is right; approximate the line as the
            // newlines between the segment start and the finding, within the
            // combined source (its stripped text tracks the original closely).
            let within = line_at(&closure.source, offset)
                .saturating_sub(line_at(&closure.source, loc.seg_start))
                + 1;
            (loc.file.to_path_buf(), within)
        }
        None => (entry_path.to_path_buf(), line_at(&closure.source, offset)),
    }
}

/// The 1-based line of byte `at` in `text`, clamped to a char boundary so a span
/// edge inside a multibyte char cannot panic the reporter.
fn line_at(text: &str, at: usize) -> usize {
    let mut upto = at.min(text.len());
    while upto > 0 && !text.is_char_boundary(upto) {
        upto -= 1;
    }
    text[..upto].bytes().filter(|&b| b == b'\n').count() + 1
}
