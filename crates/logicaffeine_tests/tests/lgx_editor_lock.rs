//! ★ LOCK — an LGX component is something the EDITOR understands.
//!
//! An author writing `## Component` spends most of their time in `State:`, `Actions:` and `View:`,
//! and the language server knew about none of it: `OwnedStmt` had no component variant at all, so
//! a whole component collapsed into `Other` and the outline, hover, go-to-definition and rename
//! all stopped at its heading. The framework compiled a page beautifully and the editor could not
//! tell you what `{count}` was.
//!
//! These are the questions an author actually asks of an editor, asked of a real server loop.

mod harness;

use harness::Harness;
use tower_lsp::lsp_types::*;

/// A component with the three things an author writes: state, an action, and a view that uses both.
const PAGE: &str = "\
## Component Badge (label: Text)
View:
    <span>{label}</span>

## Component App
State:
    Let count be 0.
Actions:
    To add:
        Set count to count + 1.
View:
    <button on-click={add}>{count}</button><Badge label=\"hi\"/>

## Main
Mount App.
";

/// Every symbol the outline reports, flattened to names.
fn names(symbols: &DocumentSymbolResponse) -> Vec<String> {
    fn walk(s: &DocumentSymbol, out: &mut Vec<String>) {
        out.push(s.name.clone());
        for c in s.children.iter().flatten() {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    match symbols {
        DocumentSymbolResponse::Nested(items) => {
            for s in items {
                walk(s, &mut out);
            }
        }
        DocumentSymbolResponse::Flat(items) => {
            out.extend(items.iter().map(|s| s.name.clone()));
        }
    }
    out
}

#[tokio::test]
async fn a_component_and_its_parts_appear_in_the_outline() {
    let mut harness = Harness::start().await;
    let uri = harness.open(PAGE).await;
    // Analysis is asynchronous: the diagnostics publish is the signal that the document has been
    // parsed and indexed, and asking before it lands gets a snapshot-less `None` rather than an
    // empty outline — a different answer with the same shape.
    let published = harness.recv_diagnostics(&uri).await;
    assert_eq!(published.diagnostics, vec![], "the page must parse cleanly");
    let symbols = harness
        .request::<request::DocumentSymbolRequest>(DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .expect("the outline must answer");
    let found = names(&symbols);
    assert!(
        found.iter().any(|n| n == "App"),
        "the component itself: {found:?}"
    );
    assert!(
        found.iter().any(|n| n == "count"),
        "its state field: {found:?}"
    );
    assert!(found.iter().any(|n| n == "add"), "its action: {found:?}");
    // ★ A PROP IS THE ONE PART THAT CANNOT COME FROM ANYWHERE ELSE. A state field reads as a
    // `Let` and an action reads as a heading, so both were already in the outline by accident of
    // how they are spelled; a prop is declared in the component's HEADING — `(label: Text)` — and
    // nothing but a component-aware index can see it. Without it this test would pass on the
    // behaviour it was written to add.
    assert!(found.iter().any(|n| n == "label"), "its prop: {found:?}");
}

/// ★ GO-TO-DEFINITION FROM INSIDE A TEMPLATE.
///
/// `{count}` in a `View:` is a use of the `count` in `State:`, and jumping between the two is the
/// single most common thing an author does in an LGX file. It did not work, and could not: the
/// whole `View:` body arrives as ONE `RawBlock` token, and references are collected by walking
/// TOKENS — so no name used only in a template was ever a reference to anything.
///
/// The positions are already known. `TExpr` carries the expression's text and its offset in the
/// template body, and the parser maps body offsets back to FILE offsets to give every `ViewNode`
/// a real span. What was missing was carrying that out to the index.
#[tokio::test]
async fn go_to_definition_from_a_hole_finds_the_state_field() {
    let mut harness = Harness::start().await;
    let uri = harness.open(PAGE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    // Line 11 is `    <button on-click={add}>{count}</button><Badge label="hi"/>`;
    // character 29 is inside the `count` of `{count}`.
    let found = harness
        .request::<request::GotoDefinition>(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 11,
                    character: 29,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .expect("a hole naming a state field must resolve");

    let line = match found {
        GotoDefinitionResponse::Scalar(loc) => loc.range.start.line,
        GotoDefinitionResponse::Array(v) => v.first().expect("at least one").range.start.line,
        GotoDefinitionResponse::Link(v) => v.first().expect("at least one").target_range.start.line,
    };
    assert_eq!(
        line, 6,
        "`{{count}}` must jump to `Let count be 0.` on line 6"
    );
}

/// The same movement, from the two other things a template names: a handler's ACTION and a placed
/// COMPONENT. A hole, an attribute expression and an element name are three different positions in
/// the template grammar, and covering only the first would answer the easiest third of the
/// question an author actually asks.
#[tokio::test]
async fn go_to_definition_reaches_an_action_and_a_placed_component() {
    let mut harness = Harness::start().await;
    let uri = harness.open(PAGE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    async fn jump(h: &mut Harness, uri: &Url, line: u32, character: u32) -> Option<u32> {
        h.request::<request::GotoDefinition>(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .map(|f| match f {
            GotoDefinitionResponse::Scalar(l) => l.range.start.line,
            GotoDefinitionResponse::Array(v) => v[0].range.start.line,
            GotoDefinitionResponse::Link(v) => v[0].target_range.start.line,
        })
    }

    // Line 11, char 23: inside `add` of `on-click={add}` — the action, declared on line 8.
    assert_eq!(
        jump(&mut harness, &uri, 11, 23).await,
        Some(8),
        "`{{add}}` must reach `To add:`"
    );
    // Line 11, char 46: inside `Badge` of `<Badge label="hi"/>` — the component, declared line 0.
    assert_eq!(
        jump(&mut harness, &uri, 11, 46).await,
        Some(0),
        "`<Badge/>` must reach `## Component Badge`"
    );
}

/// ★ HOVER, RENAME AND FIND-REFERENCES REACH INSIDE A TEMPLATE TOO.
///
/// Go-to-definition was one of FOUR features that each found "the token at the cursor" and read
/// its text as a name. Inside a `View:` that token is the whole `RawBlock`, so all four gave the
/// same wrong answer for the same reason — fixing only the one an author notices first would leave
/// three to be rediscovered separately.
#[tokio::test]
async fn hover_in_a_template_describes_the_state_field() {
    let mut harness = Harness::start().await;
    let uri = harness.open(PAGE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let hovered = harness
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 11,
                    character: 29,
                },
            },
            work_done_progress_params: Default::default(),
        })
        .await
        .expect("hovering a hole must say what it names");

    let text = match &hovered.contents {
        HoverContents::Markup(m) => m.value.clone(),
        HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
        other => panic!("unexpected hover shape: {other:?}"),
    };
    assert!(text.contains("count"), "hover must name the field: {text}");
}

#[tokio::test]
async fn rename_from_a_template_renames_the_declaration_too() {
    let mut harness = Harness::start().await;
    let uri = harness.open(PAGE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let edit = harness
        .request::<request::Rename>(RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 11,
                    character: 29,
                },
            },
            new_name: "tally".to_string(),
            work_done_progress_params: Default::default(),
        })
        .await
        .expect("renaming from a hole must be offered");

    let edits = edit.changes.expect("edits for this file");
    let all: Vec<&TextEdit> = edits.values().flatten().collect();
    assert!(
        all.iter().any(|e| e.range.start.line == 6),
        "the `State:` declaration on line 6 must be renamed: {all:?}"
    );
    assert!(
        all.iter().any(|e| e.range.start.line == 11),
        "and the `{{count}}` in the view on line 11: {all:?}"
    );
}

/// ★ COMPLETION INSIDE A TEMPLATE OFFERS THE COMPONENT'S OWN STATE.
///
/// Typing `{co` in a `View:` should offer `count`. Completion dispatches on the PREVIOUS TOKEN,
/// and inside a template that token is the whole `RawBlock` — so the one place an author types
/// most had no context at all.
///
/// ★ IT ASKS FOR A FIELD THE CURSOR IS NOT ON. `count` sits under the cursor, so a completion that
/// merely echoed the word being typed would satisfy a test that only asked for it. `draft` is the
/// other state field, nowhere near the cursor, and only a completion that actually knows what this
/// component holds can offer it.
#[tokio::test]
async fn completion_in_a_template_offers_the_components_state() {
    const PAGE: &str = "\
## Component App
State:
    Let count be 0.
    Let draft be \"\".
Actions:
    To add:
        Set count to count + 1.
View:
    <button on-click={add}>{count}</button>

## Main
Mount App.
";
    let mut harness = Harness::start().await;
    let uri = harness.open(PAGE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    // Line 8 is `    <button on-click={add}>{count}</button>`; character 30 is inside `count`.
    let got = harness
        .request::<request::Completion>(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 8,
                    character: 30,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        })
        .await
        .expect("completion must answer inside a template");

    let labels: Vec<String> = match got {
        CompletionResponse::Array(v) => v.into_iter().map(|i| i.label).collect(),
        CompletionResponse::List(l) => l.items.into_iter().map(|i| i.label).collect(),
    };
    assert!(
        labels.iter().any(|l| l == "count"),
        "the field under the cursor: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "draft"),
        "and the one that is not — a completion that only echoed the typed word would miss \
         this: {labels:?}"
    );
    // ★ AND NOT THE WHOLE STDLIB. Before this worked, a template offered every function the
    // prelude declares — around a thousand of them — so typing `{co` produced `cConj`, `cCosh`,
    // `commonRunLen` and never `count`. Useless suggestions are not a neutral default: they bury
    // the two names that were wanted, and an author learns to stop reading the list.
    assert!(
        !labels
            .iter()
            .any(|l| l == "zlibInflate" || l == "mldsaKeypair" || l == "pgScramAuth"),
        "a hole is not a statement — the stdlib does not belong in it: {} items offered",
        labels.len()
    );
    assert!(
        labels.len() < 60,
        "a hole's vocabulary is small and knowable; {} items is a firehose: {labels:?}",
        labels.len()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ THE EDITOR SAYS WHAT THE BUILD WILL SAY, WHERE THE BUILD WILL SAY IT.
//
// A page can parse, type-check and pass `largo test` on the tree-walker and still be refused by
// the browser build — and the template checker's accessibility findings are computed on every
// parse. Neither reached the editor: `largo build` was the first place either showed up. The
// language server publishes both, from the same functions the compiler and the checker run, at
// the construct each is about.
// ─────────────────────────────────────────────────────────────────────────────

/// A page the browser build refuses for ONE construct — the `bind` on an input whose `type` is
/// computed, on line 11 — while every other line of it lowers.
const REFUSED: &str = "\
## Component App
State:
    Let n be 0.
    Let draft be \"\".
    Let kind be \"text\".
Actions:
    To go:
        Set n to n + 1.
View:
    <div>
      <button id=\"a\" on-click={go}>a</button>
      <input id=\"b\" aria-label=\"b\" type={kind} bind={draft}>
    </div>

## Main
Mount App.
";

fn code_of(d: &Diagnostic) -> Option<&str> {
    match &d.code {
        Some(NumberOrString::String(c)) => Some(c.as_str()),
        _ => None,
    }
}

#[tokio::test]
async fn a_browser_tier_refusal_is_published_at_the_construct_it_refuses() {
    let mut harness = Harness::start().await;
    let uri = harness.open(REFUSED).await;
    let published = harness.recv_diagnostics(&uri).await;
    let refusal = published
        .diagnostics
        .iter()
        .find(|d| code_of(d) == Some("browser-tier-refusal"))
        .unwrap_or_else(|| {
            panic!(
                "the browser build refuses this page, so the editor must say so: {:#?}",
                published.diagnostics
            )
        });
    assert_eq!(
        refusal.severity,
        Some(DiagnosticSeverity::ERROR),
        "`largo build` stops at it"
    );
    assert_eq!(
        refusal.range.start.line, 11,
        "the refusal belongs on the refused input's line, not the component's or the file's top: \
         {refusal:#?}"
    );
    assert!(
        refusal.message.contains("`## Component App`")
            && refusal.message.contains("`type` is computed"),
        "and names the component and the construct: {}",
        refusal.message
    );
}

#[tokio::test]
async fn a_page_the_browser_can_run_carries_no_refusal() {
    // The other direction: the control is the same page with the input's type written literally.
    let mut harness = Harness::start().await;
    let uri = harness
        .open(&REFUSED.replace("type={kind}", "type=\"text\""))
        .await;
    let published = harness.recv_diagnostics(&uri).await;
    assert_eq!(
        published.diagnostics,
        vec![],
        "a page that builds is clean in the editor"
    );
}

#[tokio::test]
async fn a_template_lint_is_published_with_the_checkers_severity_and_message() {
    const PAGE: &str = "\
## Component Gallery
View:
    <div>
      <img src=\"cat.png\">
    </div>

## Main
Mount Gallery.
";
    let mut harness = Harness::start().await;
    let uri = harness.open(PAGE).await;
    let published = harness.recv_diagnostics(&uri).await;
    let lint = published
        .diagnostics
        .iter()
        .find(|d| code_of(d) == Some("lgx-image-without-alt"))
        .unwrap_or_else(|| {
            panic!(
                "an `<img>` with no `alt` is found on every parse; the editor must show it: {:#?}",
                published.diagnostics
            )
        });
    assert_eq!(
        lint.severity,
        Some(DiagnosticSeverity::WARNING),
        "a page that is hard to use is not a page that is broken"
    );
    assert!(
        lint.message.contains("alt"),
        "the checker's own message: {}",
        lint.message
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ A TEMPLATE IS PAINTED BY ITS OWN GRAMMAR.
//
// A `View:` body arrives from the lexer as ONE `RawBlock` token, and it was painted as one
// `string` — so with semantic highlighting on, the editor's own HTML colouring of every tag and
// attribute was replaced by string colour, and a hole naming a state field looked exactly like the
// text around it. The body is HTML whose braces are LOGOS; it is painted as that.
// ─────────────────────────────────────────────────────────────────────────────

mod painted {
    use logicaffeine_lsp::document::DocumentState;
    use logicaffeine_lsp::semantic_tokens::encode_document_tokens;

    /// One decoded semantic token: `(line, character, length, type)`.
    pub type Painted = (u32, u32, u32, u32);

    pub fn paint(source: &str) -> Vec<Painted> {
        let doc = DocumentState::new(source.to_string(), 1);
        let (mut line, mut character) = (0u32, 0u32);
        encode_document_tokens(&doc)
            .into_iter()
            .map(|t| {
                line += t.delta_line;
                character = if t.delta_line > 0 {
                    t.delta_start
                } else {
                    character + t.delta_start
                };
                (line, character, t.length, t.token_type)
            })
            .collect()
    }

    /// The type of the token covering `(line, character)`, or `None` when nothing is painted there.
    pub fn type_at(tokens: &[Painted], line: u32, character: u32) -> Option<u32> {
        tokens
            .iter()
            .find(|(l, c, len, _)| *l == line && *c <= character && character < c + len)
            .map(|t| t.3)
    }

    /// The column of `needle` on `line` of `source`.
    pub fn col(source: &str, line: u32, needle: &str) -> u32 {
        source
            .split('\n')
            .nth(line as usize)
            .and_then(|l| l.find(needle))
            .unwrap_or_else(|| panic!("{needle:?} is not on line {line}")) as u32
    }
}

use logicaffeine_lsp::semantic_tokens::{
    TYPE_ATTRIBUTE, TYPE_COMMENT, TYPE_FUNCTION, TYPE_KEYWORD, TYPE_MODIFIER, TYPE_OPERATOR,
    TYPE_PARAMETER, TYPE_PROPERTY, TYPE_STRING, TYPE_TAG, TYPE_TYPE,
};

#[test]
fn a_view_is_painted_as_html_with_logos_in_its_braces() {
    use painted::*;
    let t = paint(PAGE);
    // Line 11: `    <button on-click={add}>{count}</button><Badge label="hi"/>`
    let line = 11;
    let expect = |needle: &str, want: u32, what: &str| {
        let at = col(PAGE, line, needle);
        assert_eq!(
            type_at(&t, line, at),
            Some(want),
            "{what} (`{needle}` at {line}:{at}) — painted tokens on the line: {:?}",
            t.iter().filter(|x| x.0 == line).collect::<Vec<_>>()
        );
    };
    expect("button", TYPE_TAG, "an element's name is a tag");
    expect(
        "on-click",
        TYPE_KEYWORD,
        "a directive is the framework's word, not a plain attribute",
    );
    expect("add", TYPE_FUNCTION, "a handler names an action");
    expect(
        "count",
        TYPE_PROPERTY,
        "a hole naming a state field paints as that field",
    );
    expect(
        "Badge",
        TYPE_TYPE,
        "a placed component is the type it names",
    );
    expect(
        "label",
        TYPE_PARAMETER,
        "a component's attribute is the prop it binds",
    );
    expect("\"hi\"", TYPE_STRING, "a static value is a string");
    expect("{count", TYPE_OPERATOR, "a hole's brace");
    // ★ AND NOT ONE STRING OVER THE WHOLE BODY. Every assertion above could hold with a string
    // token underneath, because the finder returns the first covering token; this is the check that
    // the view's own text carries no string paint at all.
    let text_at = col(PAGE, line, "</button>");
    assert_eq!(
        type_at(&t, line, text_at),
        None,
        "markup punctuation is the editor grammar's to paint; a token there overrides it: {:?}",
        t.iter().filter(|x| x.0 == line).collect::<Vec<_>>()
    );
}

#[test]
fn a_template_tag_paints_its_words_and_resolves_its_expressions() {
    use painted::*;
    const LIST: &str = "\
## To shout (s: Text) -> Text:
    Return s.

## Component List
State:
    Let todos be [\"a\", \"b\"].
    Let title be \"Todo\".
View:
    <h1 class:big={title}>{title | shout}</h1>
    {# one row per todo #}
    <input on-keydown.enter={noop} aria-label=\"x\">
    {for each t in todos, at index i:}<p>{t}</p>{end}
Actions:
    To noop:
        Set title to title.

## Main
Mount List.
";
    let t = paint(LIST);
    let at = |line: u32, needle: &str| type_at(&t, line, col(LIST, line, needle));
    assert_eq!(
        at(8, "class:"),
        Some(TYPE_KEYWORD),
        "`class:` is a directive"
    );
    assert_eq!(
        at(8, "big"),
        Some(TYPE_ATTRIBUTE),
        "and its class name is an attribute name"
    );
    assert_eq!(at(8, "| shout"), Some(TYPE_OPERATOR), "a filter pipe");
    assert_eq!(
        type_at(&t, 8, col(LIST, 8, "| shout") + 2),
        Some(TYPE_FUNCTION),
        "a filter is a function applied to the hole"
    );
    assert_eq!(at(9, "#"), Some(TYPE_COMMENT), "a template comment");
    assert_eq!(at(10, ".enter"), Some(TYPE_MODIFIER), "an event modifier");
    assert_eq!(
        at(10, "aria-label"),
        Some(TYPE_ATTRIBUTE),
        "a plain attribute"
    );
    assert_eq!(at(11, "for each"), Some(TYPE_KEYWORD), "the tag's phrase");
    assert_eq!(
        type_at(&t, 11, col(LIST, 11, " in ") + 1),
        Some(TYPE_KEYWORD),
        "the clause word `in`"
    );
    assert_eq!(
        at(11, "todos"),
        Some(TYPE_PROPERTY),
        "the iterable resolves to the state field"
    );
    assert_eq!(
        at(11, "at index"),
        Some(TYPE_KEYWORD),
        "the clause `at index`"
    );
    assert_eq!(at(11, "end"), Some(TYPE_KEYWORD), "`{{end}}`");
}

#[test]
fn a_style_body_is_left_to_the_css_grammar() {
    use painted::*;
    const STYLED: &str = "\
## Component Card
View:
    <p>hi</p>
Style:
    p { color: red; }

## Main
Mount Card.
";
    let t = paint(STYLED);
    assert!(
        t.iter().all(|x| x.0 != 4),
        "a semantic token inside `Style:` overrides the embedded CSS grammar's colouring, and the \
         server has nothing to add about CSS: {:?}",
        t.iter().filter(|x| x.0 == 4).collect::<Vec<_>>()
    );
}

#[test]
fn every_tag_the_template_parser_accepts_is_known_to_the_editor() {
    // The editor's tag table is a second statement of the template grammar's tag words, so it is
    // held against the parser's own list, read from its source.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/lgx/template.rs"),
    )
    .expect("the template parser");
    let list = src
        .split("const TAG_WORDS: &[&str] = &[")
        .nth(1)
        .and_then(|s| s.split("];").next())
        .expect("`TAG_WORDS` is declared in template.rs");
    let words: Vec<&str> = list
        .split(',')
        .map(|w| w.trim().trim_matches('"'))
        .filter(|w| !w.is_empty())
        .collect();
    assert!(words.len() >= 11, "the tag word list shrank to {words:?}");
    let phrases: Vec<&str> = logicaffeine_lsp::template::TAGS
        .iter()
        .map(|(p, _)| *p)
        .collect();
    for w in &words {
        assert!(
            phrases.iter().any(|p| p.split(' ').next() == Some(*w)),
            "the template parser accepts `{{{w} …}}` and the editor's tag table has no phrase \
             opening with it: {phrases:?}"
        );
    }
    for p in &phrases {
        let first = p.split(' ').next().unwrap();
        assert!(
            words.contains(&first),
            "the editor offers `{p}`, which the parser does not accept"
        );
    }
}

#[test]
fn every_semantic_token_type_is_mapped_by_the_vscode_manifest() {
    // A token type the server paints and the extension does not map is painted with nothing: VS
    // Code resolves a semantic token to a theme colour through `semanticTokenScopes`, and a type
    // it has never been told about (`tag`, `attribute` — no LSP standard names them) must also be
    // DECLARED in `semanticTokenTypes`, or the client drops it.
    const STANDARD: &[&str] = &[
        "namespace",
        "type",
        "class",
        "enum",
        "interface",
        "struct",
        "typeParameter",
        "parameter",
        "variable",
        "property",
        "enumMember",
        "event",
        "function",
        "method",
        "macro",
        "keyword",
        "modifier",
        "comment",
        "string",
        "number",
        "regexp",
        "operator",
        "decorator",
    ];
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../editors/vscode/logicaffeine/package.json"),
        )
        .expect("the extension manifest"),
    )
    .expect("the manifest is JSON");
    let contributes = &manifest["contributes"];
    let scopes = contributes["semanticTokenScopes"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["language"] == "logicaffeine"))
        .map(|s| s["scopes"].clone())
        .expect("the manifest maps the language's semantic tokens to scopes");
    let declared: Vec<String> = contributes["semanticTokenTypes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for token_type in logicaffeine_lsp::semantic_tokens::TOKEN_TYPES {
        let name = token_type.as_str();
        assert!(
            scopes[name].as_array().is_some_and(|a| !a.is_empty()),
            "the server paints `{name}` and the manifest maps it to no scope"
        );
        assert!(
            STANDARD.contains(&name) || declared.iter().any(|d| d == name),
            "`{name}` is not a standard semantic token type, so the manifest must declare it in \
             `semanticTokenTypes`: {declared:?}"
        );
    }
}

/// ★ A `Style:` IS CSS, AND COMPLETION KNOWS IT.
///
/// Completion treated every raw section as a template, so a cursor inside `Style:` was offered the
/// component's state fields and `{if}` — words that mean nothing in a stylesheet. What a scoped
/// stylesheet CAN match is exactly what its own view renders, so that is what it is offered: the
/// classes, ids and elements of this component's `View:`.
#[tokio::test]
async fn completion_in_a_style_offers_the_selectors_its_view_renders() {
    const PAGE: &str = "\
## Component Row
State:
    Let done be false.
View:
    <ul class=\"list compact\"><li id=\"first\" class:done={done}>x</li></ul>
Style:
    .list { margin: 0; }

## Component Other
View:
    <section class=\"elsewhere\">y</section>

## Main
Mount Row.
";
    let mut harness = Harness::start().await;
    let uri = harness.open(PAGE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    // Line 6 is `    .list { margin: 0; }`; character 6 is inside `list`.
    let got = harness
        .request::<request::Completion>(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 6,
                    character: 6,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        })
        .await
        .expect("completion must answer inside a stylesheet");
    let labels: Vec<String> = match got {
        CompletionResponse::Array(v) => v.into_iter().map(|i| i.label).collect(),
        CompletionResponse::List(l) => l.items.into_iter().map(|i| i.label).collect(),
    };
    for want in [".list", ".compact", ".done", "#first", "ul", "li"] {
        assert!(
            labels.iter().any(|l| l == want),
            "`{want}` is something this component's view renders, so its stylesheet can match it: \
             {labels:?}"
        );
    }
    for not in ["done", "if", "for each", ".elsewhere", "section"] {
        assert!(
            !labels.iter().any(|l| l == not),
            "`{not}` is not a selector this stylesheet can match — a state field, a template tag, \
             or another component's markup (styles are scoped): {labels:?}"
        );
    }
}
