//! ★ LOCK — a diagnostic reaches the author at the line and column of their mistake. ★
//!
//! Every LGX diagnostic is decided by code that holds the offending construct — a refusal of the
//! browser build (`NotLowerable`), a finding of the template checker, a template or CSS error, a
//! name the page cannot resolve — and each carries that construct's span. What the author sees is
//! decided later, by whichever surface prints it, and a span dropped or left unmapped anywhere in
//! between renders as the top of the file, or somewhere past its end.
//!
//! This file is a CENSUS. It reads each class from its SOURCE — the `NotLowerable::at` calls in
//! the `lgx_lower` module, the `FindingKind`/`TemplateErrorKind`/`HtmlErrorKind`/`CssErrorKind` enums, the
//! LGX parser's resolution checks — and demands one row per member in `diagnostic_location/rows.rs`:
//! a page whose mistake sits on line 3 or later, the construct the diagnostic must point into, and
//! a phrase its message carries. A new member without a row fails; a row whose member is gone
//! fails. The editor half (`crates/logicaffeine_tests/tests/diagnostic_location_editor_lock.rs`) reads
//! the same table.
//!
//! A row not yet located correctly on some surface names that surface and the request it waits on
//! in its `pending` list. The list only shrinks: a pending row that starts locating correctly fails
//! until it is removed.

#[path = "diagnostic_location/rows.rs"]
mod rows;

#[path = "support/lgx_lower_source.rs"]
mod lgx_lower_source;

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_language::error::ParseError;
use rows::{Member, Reach, Row, Severity, Surface};

/// The 1-based `(line, column)` of byte `at` in `src` — the numbers an editor and a terminal show.
fn line_col(src: &str, at: usize) -> (usize, usize) {
    logicaffeine_language::diagnostic::line_col(src, at)
}

fn message_of(e: &ParseError) -> String {
    logicaffeine_language::socratic_explanation(e, &logicaffeine_language::Interner::new())
}

/// Whether byte `start` lies inside the first occurrence of `at` in `src`.
fn points_into(src: &str, at: &str, start: usize) -> bool {
    let from = src
        .find(at)
        .unwrap_or_else(|| panic!("the fixture must contain {at:?}:\n{src}"));
    start >= from && start < from + at.len()
}

fn read(relative: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

// ─────────────────────────────────────────────────────────────────────────────
// The census: each class, read from the source that defines it.
// ─────────────────────────────────────────────────────────────────────────────

/// The variants of `pub enum <name>` declared in `src`.
fn enum_variants(src: &str, name: &str) -> Vec<String> {
    let body = src
        .split(&format!("pub enum {name} {{"))
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .unwrap_or_else(|| panic!("`pub enum {name}` is declared"));
    body.lines()
        .filter_map(|l| {
            let t = l.trim();
            let head = t
                .split(|c: char| c == '(' || c == ',' || c == '{' || c == ' ')
                .next()?;
            (!head.is_empty()
                && head.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && head.chars().all(|c| c.is_ascii_alphanumeric()))
            .then(|| head.to_string())
        })
        .collect()
}

/// The first `n` words of the first string literal in `text`, or `text` itself when it holds no
/// literal (a forwarded message: `why`).
fn message_key(text: &str, n: usize) -> String {
    match text.find('"') {
        Some(open) => {
            let rest = &text[open + 1..];
            let mut out = String::new();
            let mut escaped = false;
            for c in rest.chars() {
                if escaped {
                    escaped = false;
                    continue;
                }
                match c {
                    '\\' => escaped = true,
                    '"' => break,
                    _ => out.push(c),
                }
            }
            out.split_whitespace().take(n).collect::<Vec<_>>().join(" ")
        }
        None => text.trim().to_string(),
    }
}

/// The text of the call whose `(` opens at `open` in `src`, parentheses balanced and string
/// literals skipped.
fn call_text(src: &str, open: usize) -> &str {
    let b = src.as_bytes();
    let (mut depth, mut i) = (1usize, open + 1);
    while depth > 0 && i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    &src[open + 1..i - 1]
}

/// The name of the function whose body holds byte `at`: the nearest `fn name(` before it.
fn enclosing_fn(src: &str, at: usize) -> String {
    let before = &src[..at];
    let fn_at = before.rfind("fn ").expect("inside a function");
    before[fn_at + 3..]
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap()
        .to_string()
}

/// Keys, numbered from the second occurrence within one function (`key#2`, `key#3`, …) so two
/// refusals of the same wording are two members.
fn numbered(pairs: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen: std::collections::HashMap<(String, String), usize> = Default::default();
    pairs
        .into_iter()
        .map(|(f, k)| {
            let n = seen.entry((f.clone(), k.clone())).or_insert(0);
            *n += 1;
            let key = if *n == 1 { k } else { format!("{k}#{n}") };
            (f, key)
        })
        .collect()
}

/// Every `NotLowerable::at(…)` call in the `lgx_lower` module, as `(enclosing fn, key)`.
fn not_lowerable_members() -> Vec<(String, String)> {
    let src = lgx_lower_source::source();
    let needle = "NotLowerable::at(";
    let pairs = src
        .match_indices(needle)
        .map(|(i, _)| {
            let call = call_text(&src, i + needle.len() - 1);
            let message = call.split_once(',').map_or("", |(_, m)| m);
            (enclosing_fn(&src, i), message_key(message, 4))
        })
        .collect();
    numbered(pairs)
}

/// The byte range of the body (braces included) of the function whose `{` opens at `open`: braces
/// matched with string literals, char literals and line comments skipped.
fn body_from(src: &str, open: usize) -> std::ops::Range<usize> {
    let b = src.as_bytes();
    let (mut depth, mut i) = (0usize, open);
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'\'' if i + 2 < b.len() && b[i + 2] == b'\'' => i += 2,
            b'\'' if i + 1 < b.len() && b[i + 1] == b'\\' => {
                i += 2;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return open..i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    open..b.len()
}

/// The LGX parser — `parser/component.rs` and every `parser/lgx_*.rs`, found by listing the
/// directory, so a new LGX parser file is in the census the day it is added. The expression parser
/// (`parser/mod.rs` and the rest) is not: its errors are LOGOS syntax inside a hole, located by the
/// parser that raises them.
struct ParserSource {
    files: Vec<String>,
}

impl ParserSource {
    fn load() -> Self {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/parser");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| n == "component.rs" || (n.starts_with("lgx_") && n.ends_with(".rs")))
            .collect();
        names.sort();
        assert!(
            names.iter().any(|n| n == "component.rs"),
            "the LGX parser files moved: {names:?}"
        );
        let mut files: Vec<String> = names
            .iter()
            .map(|f| read(&format!("../logicaffeine_language/src/parser/{f}")))
            .collect();
        files.extend(lgx_lower_source::files().into_iter().map(|(_, src)| src));
        ParserSource { files }
    }

    /// The body of `fn name` in the LGX parser.
    fn body(&self, name: &str) -> Option<&str> {
        let needle = format!("fn {name}(");
        self.files.iter().find_map(|src| {
            let at = src.find(&needle)?;
            let open = at + src[at..].find('{')?;
            Some(&src[body_from(src, open)])
        })
    }
}

/// The functions the LGX parser runs at the end of a parse — name resolution and structure checks
/// over the whole file — the `Head:` section's element check, and the refusal of a `View:` body
/// that does not open with markup. Every LGX-parser function they reach through `self.<fn>(…)` is
/// in the census; the expression parser they hand holes to is not (its errors are LOGOS syntax,
/// located by the parser that raises them).
const CHECK_ROOTS: &[&str] = &[
    "check_mounts_name_something",
    "check_mounted_components_need_no_props",
    "mount_the_declared_page",
    "resolve_lgx",
    "head_from_raw",
    "parse_trigger",
    "view_body_is_not_markup",
];

/// Every `ParseError` an LGX parser check can raise, as `(fn, key)`: the checks in `CHECK_ROOTS`
/// and every parser function they reach.
fn check_members() -> Vec<(String, String)> {
    let parser = ParserSource::load();
    let mut seen: Vec<String> = Vec::new();
    let mut todo: Vec<String> = CHECK_ROOTS.iter().map(|s| s.to_string()).collect();
    let mut pairs = Vec::new();
    while let Some(name) = todo.pop() {
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        let body = parser
            .body(&name)
            .unwrap_or_else(|| panic!("`fn {name}` is declared in the parser"));
        for (i, _) in body.match_indices("self.") {
            let callee: String = body[i + 5..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if body[i + 5 + callee.len()..].starts_with('(') && parser.body(&callee).is_some() {
                todo.push(callee);
            }
        }
        for (i, _) in body.match_indices("ParseError {") {
            let kind = body[i..].split("kind:").nth(1).unwrap_or("").trim_start();
            let kind = kind.trim_start_matches("crate::error::");
            let key = match kind.strip_prefix("ParseErrorKind::Custom") {
                Some(custom) => message_key(call_text(custom, 0), 4),
                None => kind
                    .split(|c: char| c == ',' || c == '{' || c == '(')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            };
            // A template error forwarded by the section that parsed it is a member of the template
            // and HTML classes, censused by its kind.
            if key == "e.message()" {
                continue;
            }
            pairs.push((name.clone(), key));
        }
    }
    numbered(pairs)
}

/// Hold one class's rows against its members as the source declares them: every member has a
/// row, and every row names a member that still exists.
fn census(class: &str, declared: Vec<String>, listed: Vec<String>, add_to: &str) {
    let missing: Vec<&String> = declared.iter().filter(|d| !listed.contains(d)).collect();
    let stale: Vec<&String> = listed.iter().filter(|l| !declared.contains(l)).collect();
    assert!(
        missing.is_empty(),
        "{class}: these members have no row in diagnostic_location/rows.rs: {missing:#?}\n\
         Each one is a diagnostic an author can meet. Add a row ({add_to}) with a page whose \
         mistake sits on line 3 or later, the construct the diagnostic must point into, and a \
         phrase its message carries — or `Reach::Unreachable` with the reason no program reaches it."
    );
    assert!(
        stale.is_empty(),
        "{class}: these rows name members the source no longer has: {stale:#?}"
    );
}

fn listed(pick: impl Fn(&Member) -> Option<String>) -> Vec<String> {
    rows::rows()
        .iter()
        .filter_map(|r| pick(&r.member))
        .collect()
}

fn pair(f: &str, k: &str) -> String {
    format!("{f} :: {k}")
}

#[test]
fn every_browser_tier_refusal_has_a_row() {
    census(
        "NotLowerable",
        not_lowerable_members()
            .into_iter()
            .map(|(f, k)| pair(&f, &k))
            .collect(),
        listed(|m| match m {
            Member::NotLowerable(f, k) => Some(pair(f, k)),
            _ => None,
        }),
        "`Member::NotLowerable(fn, key)`",
    );
}

#[test]
fn every_template_checker_finding_has_a_row() {
    census(
        "FindingKind",
        enum_variants(
            &read("../logicaffeine_language/src/lgx/check.rs"),
            "FindingKind",
        ),
        listed(|m| match m {
            Member::Finding(v) => Some(v.to_string()),
            _ => None,
        }),
        "`Member::Finding(variant)`",
    );
}

#[test]
fn every_template_error_has_a_row() {
    // `Html` and `EventModifier` carry an error of their own; their members are those enums'.
    let template: Vec<String> = enum_variants(
        &read("../logicaffeine_language/src/lgx/template.rs"),
        "TemplateErrorKind",
    )
    .into_iter()
    .filter(|v| v != "Html" && v != "EventModifier")
    .collect();
    census(
        "TemplateErrorKind",
        template,
        listed(|m| match m {
            Member::Template(v) => Some(v.to_string()),
            _ => None,
        }),
        "`Member::Template(variant)`",
    );
    let html = read("../logicaffeine_language/src/lgx/html.rs");
    census(
        "HtmlErrorKind",
        enum_variants(&html, "HtmlErrorKind"),
        listed(|m| match m {
            Member::Html(v) => Some(v.to_string()),
            _ => None,
        }),
        "`Member::Html(variant)`",
    );
    census(
        "ModifierError",
        enum_variants(&html, "ModifierError"),
        listed(|m| match m {
            Member::Modifier(v) => Some(v.to_string()),
            _ => None,
        }),
        "`Member::Modifier(variant)`",
    );
}

#[test]
fn every_css_error_has_a_row() {
    census(
        "CssErrorKind",
        enum_variants(
            &read("../logicaffeine_language/src/lgx/css.rs"),
            "CssErrorKind",
        ),
        listed(|m| match m {
            Member::Css(v) => Some(v.to_string()),
            _ => None,
        }),
        "`Member::Css(variant)`",
    );
}

#[test]
fn every_lgx_parser_check_has_a_row() {
    census(
        "LGX parser checks",
        check_members()
            .into_iter()
            .map(|(f, k)| pair(&f, &k))
            .collect(),
        listed(|m| match m {
            Member::Check(f, k) => Some(pair(f, k)),
            _ => None,
        }),
        "`Member::Check(fn, key)`",
    );
}

#[test]
fn every_copy_lint_finding_has_a_row() {
    census(
        "FindingKind (the value-semantics copy lint)",
        enum_variants(&read("../logicaffeine_language/src/lint.rs"), "FindingKind"),
        listed(|m| match m {
            Member::CopyLint(v) => Some(v.to_string()),
            _ => None,
        }),
        "`Member::CopyLint(variant)`",
    );
}

#[test]
fn every_mistake_sits_on_line_three_or_later() {
    // A mistake on line 1 is where a dropped span lands; a row there could not tell the two apart.
    for row in rows::rows() {
        let Reach::Page { source, at, .. } = &row.reach else {
            continue;
        };
        let at_byte = source
            .find(at)
            .unwrap_or_else(|| panic!("{:?}: the fixture must contain {at:?}", row.member));
        let (line, _) = line_col(source, at_byte);
        assert!(
            line >= 3,
            "{:?}: the mistake `{at}` is on line {line}; put it on line 3 or later",
            row.member
        );
    }
}

/// One surface's verdict on one row: located or not, against the row's pending list.
fn judge(row: &Row, surface: Surface, located: bool, reported: String, wrong: &mut Vec<String>) {
    let Reach::Page { pending, at, .. } = &row.reach else {
        return;
    };
    match (pending.iter().find(|(s, _)| *s == surface), located) {
        (None, true) | (Some(_), false) => {}
        (None, false) => wrong.push(format!(
            "{:?} on {surface:?}: reported at {reported}, not inside `{at}`",
            row.member
        )),
        (Some((_, waits_on)), true) => wrong.push(format!(
            "{:?} on {surface:?}: now reported inside `{at}` — the work it waited on ({waits_on}) \
             has landed, so remove {surface:?} from its `pending` list",
            row.member
        )),
    }
}

#[test]
fn the_browser_build_reports_every_mistake_where_it_is() {
    let mut wrong = Vec::new();
    for row in rows::rows() {
        let Reach::Page {
            source,
            at,
            says,
            severity,
            ..
        } = &row.reach
        else {
            continue;
        };
        match (compile_to_wasm(source), severity) {
            (Ok(_), Severity::Warning) => {}
            (Err(e), Severity::Warning) => wrong.push(format!(
                "{:?}: a warning must never stop the build, and this page was refused: {}",
                row.member,
                message_of(&e)
            )),
            (Ok(_), Severity::Error) => wrong.push(format!(
                "{:?}: the browser build ACCEPTED the page:\n{source}",
                row.member
            )),
            (Err(e), Severity::Error) => {
                let message = message_of(&e);
                if !message.contains(says) {
                    wrong.push(format!(
                        "{:?}: refused, but not for this row's reason — {message:?} does not carry {says:?}",
                        row.member
                    ));
                    continue;
                }
                let (line, col) = line_col(source, e.span.start);
                judge(
                    &row,
                    Surface::Browser,
                    points_into(source, at, e.span.start),
                    format!("{line}:{col}"),
                    &mut wrong,
                );
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "the browser build's diagnostics:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn largo_check_reports_every_warning_where_it_is() {
    let mut wrong = Vec::new();
    for row in rows::rows() {
        let Reach::Page {
            source,
            says,
            severity: Severity::Warning,
            ..
        } = &row.reach
        else {
            continue;
        };
        // Each warning-carrying class of finding has its own extractor — the template checker's
        // and the value-semantics copy lint's are different types in different modules, so a row
        // dispatches on its own `Member` to the one that actually computes it.
        let candidates: Vec<(usize, String)> = match row.member {
            Member::CopyLint(_) => logicaffeine_compile::ui_bridge::value_copy_findings_of(source)
                .into_iter()
                .map(|f| (f.span.start, f.message))
                .collect(),
            _ => logicaffeine_compile::ui_bridge::lgx_findings_of(source)
                .into_iter()
                .map(|f| (f.span.start, f.message))
                .collect(),
        };
        let Some((start, _)) = candidates.iter().find(|(_, m)| m.contains(says)) else {
            wrong.push(format!(
                "{:?}: `largo check` reports no finding carrying {says:?}: {candidates:#?}",
                row.member
            ));
            continue;
        };
        let (line, col) = line_col(source, *start);
        let into = source.find(match &row.reach {
            Reach::Page { at, .. } => at,
            Reach::Unreachable(_) => unreachable!(),
        });
        let located = into.is_some()
            && match &row.reach {
                Reach::Page { at, .. } => points_into(source, at, *start),
                Reach::Unreachable(_) => false,
            };
        judge(
            &row,
            Surface::Check,
            located,
            format!("{line}:{col}"),
            &mut wrong,
        );
    }
    assert!(
        wrong.is_empty(),
        "the template checker's warnings:\n{}",
        wrong.join("\n")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// What a refusal SAYS about where it is: the construct named as its author wrote it.
// ─────────────────────────────────────────────────────────────────────────────

/// The message the browser build refuses `src` with.
fn refusal_of(src: &str) -> String {
    message_of(
        &compile_to_wasm(src)
            .err()
            .unwrap_or_else(|| panic!("the page must be refused:\n{src}")),
    )
}

/// A view the browser build refuses on every page: a `bind` on an input whose `type` is computed, so
/// whether it writes `value` or `checked` cannot be decided before the page runs.
const REFUSED_VIEW: &str =
    "<div>\n  <input id=\"a\" aria-label=\"a\" type={kind} bind={draft}>\n</div>";

#[test]
fn a_page_is_named_by_the_header_its_author_wrote() {
    let src = rows::component(REFUSED_VIEW)
        .replacen("## Component App", "## Page /about", 1)
        .replace("## Main\nMount App.\n", "");
    let message = refusal_of(&src);
    assert!(
        message.contains("`## Page /about`"),
        "a page is named by its header: {message}"
    );
    assert!(
        !message.contains("__page"),
        "the lowered name is not a word the author ever typed: {message}"
    );
}

#[test]
fn a_component_is_named_by_its_declaration() {
    let message = refusal_of(&rows::component(REFUSED_VIEW));
    assert!(message.contains("`## Component App`"), "{message}");
}

#[test]
fn a_refusal_claims_nothing_about_another_tier() {
    // Whether the tree-walker runs a refused construct depends on the construct: a bound input of a
    // computed type it runs, a store the file never declares it does not. A sentence appended to EVERY refusal cannot know
    // which, so the refusal's own reason speaks for it.
    let message = refusal_of(&rows::component(REFUSED_VIEW));
    assert!(!message.contains("The tree-walker renders it"), "{message}");
}

#[test]
fn a_route_refusal_names_its_route() {
    let src = "# Server\n\n## Route GET /api/count -> Text:\n    Return \"1\".\n\n\
               ## Route GET /api/pair -> Seq of Int:\n    Return [1, 2].\n\n## Main\nShow \"ok\".\n";
    let message = refusal_of(src);
    assert!(
        message.contains("`## Route GET /api/pair`"),
        "a file with several routes says which: {message}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// What a driving verb's failure says on the bytecode tier: `semantics::drive`'s words, which are
// the reference tier's (`drive_message_single_source_lock`), for a selector written in the test
// and for one only known when it runs.
// ─────────────────────────────────────────────────────────────────────────────

const DRIVEN: &str = "## Component App\n\
    View:\n    <p id=\"m\">hi</p><form id=\"f\"><input id=\"d\" aria-label=\"d\"></form><select id=\"s\" aria-label=\"s\"><option>a</option></select>\n\n\
    ## Component Other\nState:\n    Let n be 0.\nActions:\n    To go:\n        Set n to n + 1.\n\
    View:\n    <button on-click={go}>other</button>\n\n";

fn vm_failure(main: &str) -> String {
    let src = format!("{DRIVEN}## Main\nMount App.\n{main}\n");
    logicaffeine_compile::compile::vm_outcome(&src)
        .error
        .unwrap_or_else(|| panic!("the statement must fail on the bytecode tier:\n{src}"))
}

#[test]
fn the_bytecode_tier_words_every_driving_failure_as_semantics_drive_does() {
    use logicaffeine_compile::semantics::drive::{no_element, nothing_answers, Verb};
    for (verb, aimed_at_nothing, unanswered, selector, key) in [
        (Verb::Click, "Click \"#none\".", "Click \"#m\".", "#m", None),
        (
            Verb::Submit,
            "Submit \"#none\".",
            "Submit \"#m\".",
            "#m",
            None,
        ),
        (
            Verb::Press,
            "Press \"Enter\" in \"#none\".",
            "Press \"Enter\" in \"#d\".",
            "#d",
            Some("Enter"),
        ),
    ] {
        assert_eq!(
            vm_failure(aimed_at_nothing),
            no_element("#none"),
            "{verb:?}"
        );
        assert_eq!(
            vm_failure(unanswered),
            nothing_answers(verb.events(), key, selector),
            "{verb:?}"
        );
    }
    // Typing into a field and choosing an option are answered by the browser itself — the control
    // takes the value — so with nothing listening they are no failure, and the test goes on, on the
    // reference and the bytecode tier alike. Aimed at nothing, they fail as every verb does.
    for (verb, aimed_at_nothing, unanswered) in [
        (
            Verb::Type,
            "Type \"x\" into \"#none\".",
            "Type \"x\" into \"#d\".",
        ),
        (
            Verb::Choose,
            "Choose \"a\" in \"#none\".",
            "Choose \"a\" in \"#s\".",
        ),
    ] {
        assert_eq!(
            vm_failure(aimed_at_nothing),
            no_element("#none"),
            "{verb:?}"
        );
        let src = format!("{DRIVEN}## Main\nMount App.\n{unanswered}\nShow \"after\".\n");
        for (tier, out) in [
            (
                "tree-walker",
                logicaffeine_compile::compile::tw_outcome(&src),
            ),
            (
                "bytecode tier",
                logicaffeine_compile::compile::vm_outcome(&src),
            ),
        ] {
            assert_eq!(
                (out.output.trim(), out.error.as_deref()),
                ("after", None),
                "{verb:?} on the {tier}:\n{src}"
            );
        }
    }
}

#[test]
fn a_selector_known_only_when_the_test_runs_is_named_in_the_failure() {
    use logicaffeine_compile::semantics::drive::{
        mount_target_missing, no_element, nothing_answers, Verb,
    };
    assert_eq!(
        vm_failure("Let s be \"#none\".\nClick s."),
        no_element("#none")
    );
    assert_eq!(
        vm_failure("Let s be \"#d\".\nLet k be \"Enter\".\nPress k in s."),
        nothing_answers(Verb::Press.events(), Some("Enter"), "#d")
    );
    let src = format!("{DRIVEN}## Main\nLet s be \"#none\".\nMount App at s.\n");
    assert_eq!(
        logicaffeine_compile::compile::vm_outcome(&src).error,
        Some(mount_target_missing("App", "#none")),
        "{src}"
    );
}
