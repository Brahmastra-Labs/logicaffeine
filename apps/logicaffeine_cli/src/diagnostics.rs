//! Every diagnostic `largo` shows, placed in the author's own file.
//!
//! A compile error, an LGX finding and a `--json` diagnostic all start as a span on whatever
//! source the compiler was handed — usually an ASSEMBLED closure, with every imported module
//! spliced in ahead of the file's own text. [`Placed`] maps that span back through the closure's
//! `SourceMap` to the file it was written in, once, and every surface renders from the result: the
//! terminal block ([`render_parse_error`], [`render_finding`]), the `logos-diagnostic/1` object
//! ([`parse_error_diagnostic`], [`finding_diagnostic`]) and the dev server's overlay
//! ([`overlay`]). None of them derives a location of its own, so `largo check`, `largo check
//! --json`, `largo serve` and `largo test` point at the same `file:line:col`.
//!
//! The envelopes (`logos-check/1`, `logos-fmt/1`, `logos-run/1`) print to stdout; the prose keeps
//! flowing to stderr so humans lose nothing.

use std::path::{Path, PathBuf};

use logicaffeine_compile::analysis::SourceMap;
use logicaffeine_language::diagnostic::{diagnostic_for, line_col, DiagSpan, Diagnostic};
use logicaffeine_language::error::ParseErrorKind;
use logicaffeine_language::lgx::check::{Finding, Severity};
use logicaffeine_language::lint::{Finding as CopyFinding, Severity as CopySeverity};
use logicaffeine_language::token::Span;
use logicaffeine_language::{socratic_explanation, Interner, ParseError};

use crate::ui::CliError;

/// A span in the author's own file: the file, its text, and the byte range within it.
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    /// The file the span is in.
    pub file: PathBuf,
    /// That file's text — the excerpt under a caret is drawn from it.
    pub text: String,
    /// The byte range, in `text`'s own coordinates.
    pub span: Span,
}

impl Placed {
    /// A span already in `file`'s coordinates.
    pub(crate) fn in_file(file: &Path, text: String, span: Span) -> Placed {
        let span = clamp(span, &text);
        Placed {
            file: file.to_path_buf(),
            text,
            span,
        }
    }

    /// A span on an assembled closure, placed in the file it was written in.
    ///
    /// An EXACT segment (the entry's own text, byte-intact) maps the whole span — file, line,
    /// column, and a caret the width of the mistake. A TRANSFORMED segment (an imported module:
    /// note-block / privacy stripping, owner-mangling) is not byte-intact, so a byte offset
    /// within it does not carry over — but its LINE does: newlines are conserved by every
    /// transform, so counting them from the segment's own start in `assembled` and landing on
    /// that line in the module's own text names the real file at its real line, with a
    /// single-character caret at the line's start standing in for the column a transform can no
    /// longer promise. Only when the segment cannot be found in `map` at all does this fall back
    /// to the assembled text under `fallback`, the file the closure was assembled from.
    pub(crate) fn locate(span: Span, assembled: &str, map: &SourceMap, fallback: &Path) -> Placed {
        if let Some(loc) = map.locate(span.start) {
            if let Ok(text) = std::fs::read_to_string(loc.file) {
                if loc.exact {
                    let width = span.end.saturating_sub(span.start);
                    let start = loc.user_offset;
                    return Placed::in_file(loc.file, text, Span::new(start, start + width));
                }
                // A module's segment starts at its own first `##` section (the entry-assembly
                // splice drops the markdown title + leading prose before it, see
                // `logicaffeine_compile::loader::module_code`), so the newlines within that prose
                // are missing from `assembled` — added back here to land on the module's OWN line.
                let dropped = newline_count(&text, header_start(&text));
                let line = dropped
                    + newline_count(assembled, span.start)
                        .saturating_sub(newline_count(assembled, loc.seg_start));
                let start = nth_line_start(&text, line);
                return Placed::in_file(loc.file, text, Span::new(start, start));
            }
        }
        Placed::in_file(fallback, assembled.to_string(), span)
    }

    /// The 1-based line and column of the span's start.
    pub(crate) fn line_col(&self) -> (usize, usize) {
        line_col(&self.text, self.span.start)
    }

    /// `file:line:col` — the form every editor and terminal opens, the file relative to the directory
    /// `largo` runs in when it is under it (as rustc names a file), so the same mistake in the same project
    /// reads the same wherever the project is.
    pub(crate) fn pointer(&self) -> String {
        let (line, col) = self.line_col();
        format!("{}:{line}:{col}", shown_path(&self.file).display())
    }

    /// The span as a `logos-diagnostic/1` span.
    fn diag_span(&self) -> DiagSpan {
        let (start_line, start_col) = line_col(&self.text, self.span.start);
        let (end_line, end_col) = line_col(&self.text, self.span.end);
        DiagSpan {
            start: self.span.start,
            end: self.span.end,
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }
}

/// `file` relative to the current directory when it is under it, else as it is.
fn shown_path(file: &Path) -> PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|here| file.strip_prefix(&here).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| file.to_path_buf())
}

/// Keep a span inside its text and on character boundaries, so no reporter can panic on it.
fn clamp(span: Span, text: &str) -> Span {
    let floor = |mut at: usize| {
        at = at.min(text.len());
        while at > 0 && !text.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let start = floor(span.start);
    let end = floor(span.end.max(span.start)).max(start);
    Span::new(start, end)
}

/// The number of newlines in `text` before byte `at` — a transform-conserved position
/// ([`Placed::locate`]'s non-exact case), since splicing, stripping and mangling all keep a
/// module's own line structure even when they change its bytes.
fn newline_count(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    text.as_bytes()[..at]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// The byte offset where 0-based line `n` of `text` starts, clamped to the text's own end when
/// `n` runs past its last line.
fn nth_line_start(text: &str, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(n - 1)
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len())
}

/// Where a module's OWN CODE starts, mirroring `logicaffeine_compile::loader::module_code`: the
/// first `## ` section, dropping the markdown title and any leading prose ahead of it (empty when
/// `text` opens with `## ` itself).
fn header_start(text: &str) -> usize {
    text.find("\n## ").map(|i| i + 1).unwrap_or(0)
}

/// The error with its span moved into the placed file.
fn placed_error(pe: &ParseError, at: &Placed) -> ParseError {
    ParseError {
        kind: pe.kind.clone(),
        span: at.span,
    }
}

/// The caret excerpt of `at`, without `display_with_source`'s own `error:` headline.
fn excerpt(pe: &ParseError, at: &Placed) -> String {
    placed_error(pe, at)
        .display_with_source(&at.text)
        .lines()
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A compile error as the terminal shows it: the Socratic explanation, the caret excerpt, and the
/// `--> file:line:col` an editor opens.
pub(crate) fn render_parse_error(pe: &ParseError, at: &Placed) -> String {
    let explanation = socratic_explanation(&placed_error(pe, at), &Interner::new());
    format!("{explanation}\n{}\n  --> {}", excerpt(pe, at), at.pointer())
}

/// A compile error as a `logos-diagnostic/1` object naming its file.
pub(crate) fn parse_error_diagnostic(pe: &ParseError, at: &Placed) -> Diagnostic {
    let file = at.file.display().to_string();
    diagnostic_for(
        &placed_error(pe, at),
        &at.text,
        Some(&file),
        &Interner::new(),
    )
}

/// The word a finding is reported under.
pub(crate) fn finding_severity(f: &Finding) -> &'static str {
    match f.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// A finding's stable code: its kind's name in kebab case (`ImageWithoutAlt` →
/// `image-without-alt`), so a new kind is reported under its own name the moment it exists.
pub(crate) fn finding_code(f: &Finding) -> String {
    let debug = format!("{:?}", f.kind);
    let name = debug
        .split(|c: char| c == '(' || c == ' ' || c == '{')
        .next()
        .unwrap_or(&debug);
    let mut code = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                code.push('-');
            }
            code.push(c.to_ascii_lowercase());
        } else {
            code.push(c);
        }
    }
    code
}

/// An LGX finding as the terminal shows it — its message, the markup it is about, and where.
pub(crate) fn render_finding(f: &Finding, at: &Placed) -> String {
    let carrier = ParseError {
        kind: ParseErrorKind::Custom(f.message.clone()),
        span: f.span,
    };
    format!(
        "{}\n{}\n  --> {}",
        f.message,
        excerpt(&carrier, at),
        at.pointer()
    )
}

/// An LGX finding as a `logos-diagnostic/1` object naming its file.
pub(crate) fn finding_diagnostic(f: &Finding, at: &Placed) -> Diagnostic {
    Diagnostic {
        code: finding_code(f),
        severity: finding_severity(f).to_string(),
        file: Some(at.file.display().to_string()),
        span: at.diag_span(),
        message: f.message.clone(),
        explanation: String::new(),
        suggestion: None,
        quickfix: None,
        docs: None,
        related: Vec::new(),
    }
}

/// The word a value-semantics copy warning is reported under — mirrors `finding_severity`'s own
/// convention for its own, separate `Finding` type.
pub(crate) fn copy_finding_severity(f: &CopyFinding) -> &'static str {
    match f.severity {
        CopySeverity::Warning => "warning",
    }
}

/// A copy-lint finding's stable code: its kind's name in kebab case, the same convention
/// `finding_code` uses (a new kind is reported under its own name the moment it exists).
pub(crate) fn copy_finding_code(f: &CopyFinding) -> String {
    let debug = format!("{:?}", f.kind);
    let name = debug
        .split(|c: char| c == '(' || c == ' ' || c == '{')
        .next()
        .unwrap_or(&debug);
    let mut code = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                code.push('-');
            }
            code.push(c.to_ascii_lowercase());
        } else {
            code.push(c);
        }
    }
    code
}

/// A value-semantics copy warning as the terminal shows it — its message, the mutation it is
/// about, and where.
pub(crate) fn render_copy_finding(f: &CopyFinding, at: &Placed) -> String {
    let carrier = ParseError {
        kind: ParseErrorKind::Custom(f.message.clone()),
        span: f.span,
    };
    format!(
        "{}\n{}\n  --> {}",
        f.message,
        excerpt(&carrier, at),
        at.pointer()
    )
}

/// A value-semantics copy warning as a `logos-diagnostic/1` object naming its file.
pub(crate) fn copy_finding_diagnostic(f: &CopyFinding, at: &Placed) -> Diagnostic {
    Diagnostic {
        code: copy_finding_code(f),
        severity: copy_finding_severity(f).to_string(),
        file: Some(at.file.display().to_string()),
        span: at.diag_span(),
        message: f.message.clone(),
        explanation: String::new(),
        suggestion: None,
        quickfix: None,
        docs: None,
        related: Vec::new(),
    }
}

/// The dev server's error overlay for a compile error, drawn against the file it is in and naming
/// it. `reload` is the live-reload client the overlay carries so a fix snaps back.
pub(crate) fn overlay(pe: &ParseError, at: &Placed, reload: &str) -> String {
    let name = at
        .file
        .file_name()
        .map(|f| f.to_string_lossy().into_owned());
    logicaffeine_compile::web_host::error_overlay_html_in(
        &placed_error(pe, at),
        &at.text,
        reload,
        name.as_deref(),
    )
}

/// Why an entry program failed to become one, rendered — the same block `largo check` prints.
///
/// For the callers (`largo serve`, `largo build`) that must know what a project IS before they
/// can act, and so parse its entry first: when that parse fails, the author gets the compiler's
/// own diagnostic at the line, not a guess at the project's shape.
pub(crate) fn entry_failure(
    entry: &Path,
    dep_roots: Vec<logicaffeine_compile::loader::DepRoot>,
) -> CliError {
    match crate::compile::compile_project_with_deps(entry, dep_roots.clone()) {
        Err(e) => compile_error(&e, entry, &dep_roots),
        Ok(_) => CliError::new(format!(
            "{} could not be read as a program",
            entry.display()
        )),
    }
}

/// The front end every command that runs or emits a program passes it through before choosing a
/// backend: a program it rejects is reported once, in the words `largo` uses whichever backend was asked
/// for, and no backend sees it. A program that reaches a module it never imported is refused here too.
pub(crate) fn front_end(
    entry: &Path,
    dep_roots: &[logicaffeine_compile::loader::DepRoot],
) -> Result<(), Box<dyn std::error::Error>> {
    use logicaffeine_compile::compile::{CompileError, ProjectScopeError};
    match logicaffeine_compile::compile::enforce_project_scope_with_deps(entry, dep_roots.to_vec())
    {
        Ok(()) => Ok(()),
        Err(ProjectScopeError::Parse(_)) => {
            match crate::compile::compile_project_with_deps(entry, dep_roots.to_vec()) {
                Err(e @ CompileError::Parse(_)) => Err(compile_error(&e, entry, dep_roots).into()),
                _ => Err(entry_failure(entry, dep_roots.to_vec()).into()),
            }
        }
        Err(refused) => Err(refused.into()),
    }
}

/// A `CompileError` from the entry's Rust pipeline, rendered: a parse error at its line, in
/// whichever file it is actually in.
pub(crate) fn compile_error(
    e: &logicaffeine_compile::compile::CompileError,
    entry: &Path,
    dep_roots: &[logicaffeine_compile::loader::DepRoot],
) -> CliError {
    match e {
        logicaffeine_compile::compile::CompileError::Parse(pe) => {
            CliError::new(rejection(pe, located_in_project(pe, entry, dep_roots)))
        }
        other => CliError::new(other.to_string()),
    }
}

/// A rejected program's diagnostic as the terminal shows it, placed at `at`, an unknown call located at
/// its call site.
fn rejection(pe: &ParseError, at: Placed) -> String {
    match locate_unknown_call(pe, &at.text) {
        Some(located) => {
            let at = Placed::in_file(&at.file, at.text, located.span);
            render_parse_error(&located, &at)
        }
        None => render_parse_error(pe, &at),
    }
}

/// Everything `largo` writes to standard error for a program the compiler rejects with `pe`, whose span is
/// in `text`, the text of `file`: the `error:` block with the explanation, the excerpt and the
/// `--> file:line:col`, without terminal styles. Every backend `largo` drives reports a rejected program in
/// exactly these bytes, and the tier-parity census holds the backends it drives in-process to them too.
pub fn parse_error_report(file: &Path, text: &str, pe: &ParseError) -> String {
    let message = rejection(pe, Placed::in_file(file, text.to_string(), pe.span));
    crate::ui::plain(&crate::ui::error_block(&message, None))
}

/// A parse error from the entry's Rust pipeline, placed through the SAME closure the pipeline
/// compiled: `compile_project_with_deps` assembles every imported module ahead of the entry into
/// one combined source before parsing, so a span on that source is the entry's own coordinates
/// only when nothing is imported. Rebuilding the identical closure here recovers the real file and
/// line — an import's own mistake lands in that module, and the entry's own mistake lands
/// correctly even with imports spliced ahead of it — through the same [`Placed::locate`] every
/// other surface uses. Falls back to the entry's raw text when the closure itself will not
/// assemble (a different failure than the one being reported).
fn located_in_project(
    pe: &ParseError,
    entry: &Path,
    dep_roots: &[logicaffeine_compile::loader::DepRoot],
) -> Placed {
    match crate::closure::assemble_closure_with_deps(entry, dep_roots.to_vec()) {
        Ok(closure) => Placed::locate(pe.span, &closure.source, &closure.source_map, entry),
        Err(_) => entry_placement(pe, entry),
    }
}

/// The entry-file placement of a parse error, treating its span as the entry's own raw-text
/// coordinates — correct only when the entry imports nothing. The fallback of
/// [`located_in_project`] for when the closure cannot be reassembled.
pub(crate) fn entry_placement(pe: &ParseError, entry: &Path) -> Placed {
    let text = std::fs::read_to_string(entry).unwrap_or_default();
    Placed::in_file(entry, text, pe.span)
}

/// The entry's parse error as a `logos-diagnostic/1` object, with an unknown call located at its
/// call site.
pub(crate) fn entry_parse_diagnostic(
    pe: &ParseError,
    entry: &Path,
    dep_roots: &[logicaffeine_compile::loader::DepRoot],
) -> Diagnostic {
    let at = located_in_project(pe, entry, dep_roots);
    match locate_unknown_call(pe, &at.text) {
        Some(located) => {
            let at = Placed::in_file(&at.file, at.text, located.span);
            parse_error_diagnostic(&located, &at)
        }
        None => parse_error_diagnostic(pe, &at),
    }
}

/// The byte ranges of `source` that are prose rather than code: everything before the first `##`
/// header, every `## Note` block, and every backtick span — the places a name can be MENTIONED
/// without being called.
fn prose_ranges(source: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut prose_from: Option<usize> = Some(0);
    for line in source.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("## ") || t == "##\n" || t == "##" {
            if let Some(from) = prose_from.take() {
                out.push(from..offset);
            }
            if t.starts_with("## Note") {
                prose_from = Some(offset);
            }
        }
        let mut in_tick: Option<usize> = None;
        for (i, c) in line.char_indices() {
            if c == '`' {
                match in_tick.take() {
                    Some(from) => out.push(offset + from..offset + i + 1),
                    None => in_tick = Some(i),
                }
            }
        }
        offset += line.len();
    }
    if let Some(from) = prose_from {
        out.push(from..offset);
    }
    out
}

/// If `pe` is a span-less "Unknown function: <name>" diagnostic, a copy whose span points at the
/// name's CALL site in `source`. The AST's `Expr::Call` carries no span, so the checker emits this
/// diagnostic span-less; the location is recovered from the SAME text the caret is drawn against,
/// skipping prose — a `## Note` or a backticked mention names the function without calling it.
/// Prefers a call position (`name(` or `name of`); `None` for any other error or a name not found.
pub(crate) fn locate_unknown_call(pe: &ParseError, source: &str) -> Option<ParseError> {
    let msg = match &pe.kind {
        ParseErrorKind::Custom(s) => s.as_str(),
        _ => return None,
    };
    let name = msg.strip_prefix("Unknown function: ")?.trim();
    if name.is_empty() {
        return None;
    }
    let prose = prose_ranges(source);
    let in_prose = |at: usize| prose.iter().any(|r| r.contains(&at));
    let bytes = source.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut fallback: Option<usize> = None;
    let mut from = 0;
    while let Some(rel) = source[from..].find(name) {
        let at = from + rel;
        let end = at + name.len();
        let bounded =
            (at == 0 || !is_ident(bytes[at - 1])) && (end >= bytes.len() || !is_ident(bytes[end]));
        if bounded && !in_prose(at) {
            let mut k = end;
            while k < bytes.len() && bytes[k] == b' ' {
                k += 1;
            }
            if k < bytes.len() && (bytes[k] == b'(' || source[k..].starts_with("of ")) {
                return Some(ParseError {
                    kind: pe.kind.clone(),
                    span: Span::new(at, end),
                });
            }
            fallback.get_or_insert(at);
        }
        from = end;
    }
    fallback.map(|at| ParseError {
        kind: pe.kind.clone(),
        span: Span::new(at, at + name.len()),
    })
}

/// A failure that belongs to a whole file rather than a line in it — a module that will not
/// assemble, a backend error with no source position — as a `logos-diagnostic/1` object that
/// still names the file.
pub(crate) fn file_failure(file: &Path, code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: "error".to_string(),
        file: Some(file.display().to_string()),
        span: DiagSpan {
            start: 0,
            end: 0,
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 1,
        },
        message: message.lines().next().unwrap_or(message).to_string(),
        explanation: message.to_string(),
        suggestion: None,
        quickfix: None,
        docs: None,
        related: Vec::new(),
    }
}

/// The `logos-check/1` envelope as a single pretty JSON document.
pub(crate) fn check_envelope(ok: bool, diagnostics: &[Diagnostic]) -> String {
    let doc = serde_json::json!({
        "schema": "logos-check/1",
        "ok": ok,
        "diagnostics": diagnostics,
    });
    serde_json::to_string_pretty(&doc).expect("envelope serializes")
}

/// The `logos-fmt/1` envelope.
pub(crate) fn fmt_envelope(clean: bool, unformatted: &[String]) -> String {
    let doc = serde_json::json!({
        "schema": "logos-fmt/1",
        "clean": clean,
        "unformatted": unformatted,
    });
    serde_json::to_string_pretty(&doc).expect("envelope serializes")
}

/// The `logos-run/1` envelope: one run's outcome, output, and step account.
pub(crate) fn run_envelope(
    outcome: &str,
    lines: &[String],
    error: Option<&str>,
    steps_used: Option<u64>,
) -> String {
    let doc = serde_json::json!({
        "schema": "logos-run/1",
        "outcome": outcome,
        "lines": lines,
        "error": error,
        "steps_used": steps_used,
    });
    serde_json::to_string_pretty(&doc).expect("envelope serializes")
}

/// A `logos-check/1` envelope for a failure that is not a compile error — the cargo phase of
/// `largo build --json`. The raw tool output rides in `explanation`, never dropped. There is no
/// source location to give, so `file` is `null`.
pub(crate) fn build_failure_envelope(message: &str, explanation: &str) -> String {
    let doc = serde_json::json!({
        "schema": "logos-check/1",
        "ok": false,
        "diagnostics": [{
            "code": "build-failure",
            "severity": "error",
            "file": null,
            "span": {"start": 0, "end": 0, "start_line": 1, "start_col": 1,
                      "end_line": 1, "end_col": 1},
            "message": message,
            "explanation": explanation,
            "suggestion": null,
            "quickfix": null,
            "docs": null,
            "related": [],
        }],
    });
    serde_json::to_string_pretty(&doc).expect("envelope serializes")
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_cli/diagnostics/tests.rs"]
mod tests;
