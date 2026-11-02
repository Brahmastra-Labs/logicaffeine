//! The canonical-teaching rule set: what every LOGOS web snippet a person can read must satisfy.
//!
//! One module, two appliers. `crates/logicaffeine_tests/tests/canonical_teaching_lock.rs` reads the
//! file surfaces (`docs/`, `README.md`, `LOGOS_QUICKGUIDE.md`, the scaffold's `AGENTS.md`,
//! `llms-full.txt`, `examples/`); `crates/logicaffeine_tests/tests/web_canonical_teaching_lock.rs` reads the
//! in-memory registries the web app ships (the guide's sections and examples, the Studio examples).
//! Both include this file, so a surface cannot be held to a weaker rule than another.
//!
//! The canon: a page is `## Page /path` and mounts itself; a part is `## Component Name (props)`;
//! shared state is `## Store`; a server is `## Route`s and `Serve on port N.` `Mount` appears in a
//! lesson only inside a `## Test`, or in a fence explicitly marked [`MULTI_ROOT`].
//!
//! Every rule is a [`Rule`] variant, so a failure names the rule it broke and the census of rules is
//! the enum itself.

#![allow(dead_code)]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_language::ast::stmt::{Stmt, UiStmt};
use logicaffeine_language::{BlockType, Interner, Lexer, TokenType};

/// The fence tag of a LOGOS program the documentation harness compiles to a native target.
pub const TAG_LOGOS: &str = "logos";

/// The fence tag of a LOGOS web page: parsed, mounted, rendered on every in-process tier, compiled
/// for the browser, and its `## Test` blocks run.
pub const TAG_PAGE: &str = "logos-dom";

/// The fence attribute (```` ```logos-dom multi-root ````) that marks a lesson about a program choosing
/// its own roots — the one place a `Mount` outside a `## Test` is the thing being taught.
pub const MULTI_ROOT: &str = "multi-root";

/// Every way a teaching snippet can fall short of the canon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rule {
    /// A fence holding LOGOS carries no LOGOS tag, so no harness runs it.
    UntaggedLogos,
    /// A fence holding a page, store, error page or directive is not tagged [`TAG_PAGE`].
    PageNotTaggedAsPage,
    /// A [`TAG_PAGE`] fence holds no page, component, store, error page or directive.
    NotAPage,
    /// A form of the retired signal dialect.
    LegacyDialect,
    /// `Mount` outside a `## Test` in a lesson not marked [`MULTI_ROOT`].
    MountOutsideTest,
    /// The snippet does not parse.
    DoesNotParse,
    /// The reference tier (the tree-walker) fails running it.
    FailsOnTheReferenceTier,
    /// The bytecode VM fails running it.
    FailsOnTheVm,
    /// The tree-walker and the VM render different documents.
    TiersDisagree,
    /// The browser tier refuses it.
    DoesNotCompileForTheBrowser,
    /// A `## Test` it carries fails.
    ATestFails,
    /// Nothing is mounted: no page renders and no test mounts one.
    RendersNothing,
    /// A program fence does not compile on the native tier.
    DoesNotCompile,
}

impl Rule {
    /// The stable kebab-case name a failure and a pending entry use.
    pub fn name(self) -> &'static str {
        match self {
            Rule::UntaggedLogos => "untagged-logos",
            Rule::PageNotTaggedAsPage => "page-not-tagged-as-page",
            Rule::NotAPage => "not-a-page",
            Rule::LegacyDialect => "legacy-dialect",
            Rule::MountOutsideTest => "mount-outside-test",
            Rule::DoesNotParse => "does-not-parse",
            Rule::FailsOnTheReferenceTier => "fails-on-the-reference-tier",
            Rule::FailsOnTheVm => "fails-on-the-vm",
            Rule::TiersDisagree => "tiers-disagree",
            Rule::DoesNotCompileForTheBrowser => "does-not-compile-for-the-browser",
            Rule::ATestFails => "a-test-fails",
            Rule::RendersNothing => "renders-nothing",
            Rule::DoesNotCompile => "does-not-compile",
        }
    }
}

/// One piece of teaching code: a fenced block, an inline code span, or a whole example source.
#[derive(Debug, Clone)]
pub struct Snippet {
    /// Where it lives: a repository path, or a registry name for an in-memory surface.
    pub surface: String,
    /// The 1-based line the body starts on (0 for a whole-source snippet).
    pub line: usize,
    /// The fence's first info-string word (`""` for an untagged fence).
    pub tag: String,
    /// The fence's remaining info-string words.
    pub attrs: Vec<String>,
    pub body: String,
}

impl Snippet {
    /// A whole source that is a page by construction — a Studio example, a guide example, a file
    /// in an example project's `pages/`.
    pub fn page_source(surface: impl Into<String>, body: impl Into<String>) -> Snippet {
        Snippet {
            surface: surface.into(),
            line: 0,
            tag: TAG_PAGE.to_string(),
            attrs: Vec::new(),
            body: body.into(),
        }
    }

    pub fn location(&self) -> String {
        if self.line == 0 {
            self.surface.clone()
        } else {
            format!("{}:{}", self.surface, self.line)
        }
    }
}

/// One broken rule, with what the rule saw.
#[derive(Debug, Clone)]
pub struct Violation {
    pub rule: Rule,
    pub location: String,
    pub detail: String,
    /// The snippet's first line, the stable half of a pending entry's identity.
    pub first_line: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} — {}",
            self.rule.name(),
            self.location,
            self.detail
        )
    }
}

/// Every fenced block in a markdown text. Fences open with three or more backticks and close with a
/// line of backticks; the info string is split into a tag and attributes.
pub fn fences(surface: &str, text: &str) -> Vec<Snippet> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if let Some(info) = t.strip_prefix("```") {
            let mut words = info.split_whitespace().map(str::to_string);
            let tag = words.next().unwrap_or_default();
            let attrs: Vec<String> = words.collect();
            let start = i + 2;
            let mut body = String::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                body.push_str(lines[i]);
                body.push('\n');
                i += 1;
            }
            out.push(Snippet {
                surface: surface.to_string(),
                line: start,
                tag,
                attrs,
                body,
            });
        }
        i += 1;
    }
    out
}

/// Every inline code span (`` `…` ``) outside fences, with its line.
pub fn inline_spans(surface: &str, text: &str) -> Vec<Snippet> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for (n, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('`') else { break };
            let span = &after[..close];
            if !span.is_empty() {
                out.push(Snippet {
                    surface: surface.to_string(),
                    line: n + 1,
                    tag: String::from("inline"),
                    attrs: Vec::new(),
                    body: span.to_string(),
                });
            }
            rest = &after[close + 1..];
        }
    }
    out
}

/// The block headers the real lexer reads in `body`, as the lexer classifies them. Prose headers
/// (`## Note`, `## Example`, `## Logic`, and any unknown `## Heading`) are not LOGOS and are left out;
/// a near-miss of a code header is kept, because it is a typo of one.
pub fn code_headers(body: &str) -> Vec<BlockType> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(body, &mut interner);
    lexer
        .tokenize()
        .into_iter()
        .filter_map(|t| match t.kind {
            TokenType::BlockHeader { block_type } => Some(block_type),
            _ => None,
        })
        .filter(|b| !matches!(b, BlockType::Note | BlockType::Example | BlockType::Logic))
        .collect()
}

/// Whether a header declares a browser construct — something only a document can run.
fn is_browser_header(b: &BlockType) -> bool {
    matches!(
        b,
        BlockType::Page { .. }
            | BlockType::Store
            | BlockType::ErrorPage { .. }
            | BlockType::Directive
    )
}

/// Whether a header declares a piece of a web page.
fn is_lgx_header(b: &BlockType) -> bool {
    is_browser_header(b) || matches!(b, BlockType::Component)
}

/// The retired signal dialect, one detector per form: `(the form, whether this line is it)`.
///
/// The forms are matched on the text rather than the AST because the dialect is leaving the parser: a
/// snippet written in it will stop parsing, and the teaching surface must say which form it used
/// rather than only that it no longer parses.
pub const LEGACY_FORMS: &[(&str, fn(&str) -> bool)] = &[
    (
        "`Render:` (a view is `View:` inside a `## Page` or `## Component`)",
        |l| {
            let t = l.trim_start();
            t.starts_with("Render:") || t.starts_with("Render into ")
        },
    ),
    (
        "`Let x be a signal …` (state is a plain `Let` in `State:`)",
        |l| l.contains(" be a signal"),
    ),
    (
        "`Whenever … run …` (a reaction is a `When … changes:` trigger in `Actions:`)",
        |l| {
            let t = l.trim_start();
            t.starts_with("Whenever ") && t.contains(" run ")
        },
    ),
    ("`Put … in …` (a view is markup in `View:`)", |l| {
        let t = l.trim_start();
        t.starts_with("Put ")
            && (t.contains(" in ") || t.contains(" into "))
            && !t.contains(" then ")
    }),
];

/// The legacy forms `text` uses, one entry per offending line.
pub fn legacy_forms(text: &str) -> Vec<(usize, &'static str)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for (form, is) in LEGACY_FORMS {
            if is(line) {
                out.push((n, *form));
            }
        }
    }
    out
}

fn violation(s: &Snippet, rule: Rule, detail: impl Into<String>) -> Violation {
    Violation {
        rule,
        location: s.location(),
        detail: detail.into(),
        first_line: s
            .body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string(),
    }
}

/// Every rule a fenced block or whole source breaks.
pub fn check(s: &Snippet) -> Vec<Violation> {
    let mut out = Vec::new();
    for (n, form) in legacy_forms(&s.body) {
        out.push(violation(
            s,
            Rule::LegacyDialect,
            format!("line {} of the snippet uses {form}", n + 1),
        ));
    }
    if s.tag == "inline" {
        return out;
    }
    let headers = code_headers(&s.body);
    let is_logos_tag = s.tag == TAG_LOGOS || s.tag == TAG_PAGE;
    if !headers.is_empty() && !is_logos_tag {
        out.push(violation(
            s,
            Rule::UntaggedLogos,
            format!(
                "a fence tagged {:?} holds LOGOS, so no harness runs it — tag it `{TAG_PAGE}` (a page) or `{TAG_LOGOS}` (a program)",
                s.tag
            ),
        ));
        return out;
    }
    if headers.iter().any(is_browser_header) && s.tag != TAG_PAGE {
        out.push(violation(
            s,
            Rule::PageNotTaggedAsPage,
            format!("a page, store, error page or directive belongs in a `{TAG_PAGE}` fence"),
        ));
        return out;
    }
    if s.tag != TAG_PAGE {
        return out;
    }
    if !headers.iter().any(is_lgx_header) {
        out.push(violation(
            s,
            Rule::NotAPage,
            "holds no `## Page`, `## Component`, `## Store`, `## Error page` or `## Directive`",
        ));
        return out;
    }
    out.extend(check_page(s));
    out
}

/// The page battery: parse, the `Mount` rule, both in-process tiers, the browser compile, the tests.
fn check_page(s: &Snippet) -> Vec<Violation> {
    let mut out = Vec::new();
    let body = s.body.as_str();

    let parsed = with_parsed_program(body, |parsed, interner| match parsed {
        Err(advice) => Err(advice),
        Ok((stmts, _, _)) => Ok(stray_mounts(stmts, interner)),
    });
    let stray = match parsed {
        Err(advice) => {
            out.push(violation(s, Rule::DoesNotParse, advice));
            return out;
        }
        Ok(stray) => stray,
    };
    if !s.attrs.iter().any(|a| a == MULTI_ROOT) {
        for name in stray {
            out.push(violation(
                s,
                Rule::MountOutsideTest,
                format!("`Mount {name}` outside a `## Test` — a page is `## Page /path` and mounts itself"),
            ));
        }
    }

    dom::reset();
    let tw = tw_outcome(body);
    let tw_doc = dom::render();
    if let Some(e) = tw.error {
        out.push(violation(s, Rule::FailsOnTheReferenceTier, e));
    }

    dom::reset();
    let vm = vm_outcome(body);
    let vm_doc = dom::render();
    dom::reset();
    match vm.error {
        Some(e) => out.push(violation(s, Rule::FailsOnTheVm, e)),
        None if vm_doc != tw_doc => out.push(violation(
            s,
            Rule::TiersDisagree,
            format!("tree-walker: {tw_doc}\n        vm:          {vm_doc}"),
        )),
        None => {}
    }

    if let Err(e) = compile_to_wasm(body) {
        out.push(violation(
            s,
            Rule::DoesNotCompileForTheBrowser,
            format!("{e:?}"),
        ));
    }

    let mut a_test_mounted = false;
    match run_tests(body, &TestConfig::default()) {
        Err(e) => out.push(violation(
            s,
            Rule::ATestFails,
            format!("the tests could not run: {e}"),
        )),
        Ok(report) => {
            for o in &report.outcomes {
                if o.verdict == Verdict::Failed {
                    out.push(violation(
                        s,
                        Rule::ATestFails,
                        format!("`{}`: {}", o.name, o.failures.join("; ")),
                    ));
                } else if o.verdict == Verdict::Passed && o.mounted {
                    a_test_mounted = true;
                }
            }
        }
    }
    dom::reset();

    let error_pages: Vec<u16> = code_headers(body)
        .into_iter()
        .filter_map(|b| match b {
            BlockType::ErrorPage { status } => Some(status),
            _ => None,
        })
        .collect();
    let mut error_pages_render = !error_pages.is_empty();
    for status in &error_pages {
        match render_error_page(body, *status) {
            Ok(markup) if !markup.trim().is_empty() => {}
            Ok(_) => {
                error_pages_render = false;
                out.push(violation(
                    s,
                    Rule::RendersNothing,
                    format!("`## Error page {status}` renders empty through the router"),
                ));
            }
            Err(e) => {
                error_pages_render = false;
                out.push(violation(
                    s,
                    Rule::FailsOnTheReferenceTier,
                    format!("`## Error page {status}`: {e}"),
                ));
            }
        }
    }

    if tw_doc == "<body></body>" && !a_test_mounted && !error_pages_render {
        out.push(violation(
            s,
            Rule::RendersNothing,
            "nothing is mounted — a lesson is a `## Page`, or a `## Test` that mounts the component it teaches",
        ));
    }
    out
}

/// The markup the router answers with for `status` — an `## Error page` is served, not mounted, so
/// it renders through a request that reaches it: a URL nothing matches for `404`, a method the one
/// probe route does not take for `405`.
fn render_error_page(body: &str, status: u16) -> Result<String, String> {
    let (method, url) = match status {
        404 => ("GET", "/__canonical_no_such_url"),
        405 => ("POST", "/__canonical_probe"),
        other => {
            return Err(format!(
                "no request reaches a `{other}` page — the router answers 404 and 405 with one"
            ))
        }
    };
    let probe = format!(
        "{body}\n## Route GET /__canonical_probe -> Text:\n    Return \"probe\".\n\n## Main\nShow route(\"{method}\", \"{url}\").\n"
    );
    let out = tw_outcome(&probe);
    match out.error {
        Some(e) => Err(e),
        None => Ok(out.output),
    }
}

/// The components a program's own statements mount — every `Mount` outside a `## Test`, except the
/// mount a `## Page` gives itself.
fn stray_mounts(stmts: &[Stmt], interner: &Interner) -> Vec<String> {
    stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Ui(UiStmt::Mount { component, .. }) => {
                let name = interner.resolve(*component);
                (!name.starts_with("__page_")).then(|| name.to_string())
            }
            _ => None,
        })
        .collect()
}

/// A violation the owning lane has been asked to fix: `(surface, rule, a line the snippet contains, the
/// request)`. A pending entry must keep matching a real violation — a fixed one fails until it is
/// deleted, so the list only shrinks.
pub struct Pending {
    pub surface: &'static str,
    pub rule: Rule,
    pub needle: &'static str,
    pub request: &'static str,
}

/// Hold `found` against `pending`: every violation is pending or it fails, and every pending entry
/// still matches a violation or it fails. Returns the failure report, empty when both hold.
pub fn reconcile(found: &[Violation], pending: &[Pending], snippets: &[Snippet]) -> Vec<String> {
    let body_of = |loc: &str| {
        snippets
            .iter()
            .find(|s| s.location() == loc)
            .map(|s| s.body.as_str())
            .unwrap_or("")
    };
    let matches = |v: &Violation, p: &Pending| {
        v.rule == p.rule
            && (v.location == p.surface || v.location.starts_with(&format!("{}:", p.surface)))
            && body_of(&v.location).contains(p.needle)
    };
    let mut report = Vec::new();
    for v in found {
        if !pending.iter().any(|p| matches(v, p)) {
            report.push(format!("{v}"));
        }
    }
    for p in pending {
        if !found.iter().any(|v| matches(v, p)) {
            report.push(format!(
                "[pending-fixed] {} no longer breaks `{}` at `{}` — the request ({}) is done: delete this pending entry",
                p.surface,
                p.rule.name(),
                p.needle,
                p.request
            ));
        }
    }
    report
}
