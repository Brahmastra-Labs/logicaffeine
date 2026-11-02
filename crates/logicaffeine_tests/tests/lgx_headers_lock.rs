//! ★ LOCK — the lexer captures LGX sections RAW and reads the new block headers. ★
//!
//! HTML and CSS are not LOGOS tokens. A `View:` section whose body is markup (its first
//! line starts with `<` or `{`) and every `Style:` section are captured VERBATIM by the
//! lexer as one `RawBlock` token — base indentation stripped, inner indentation and blank
//! lines preserved, and the body's byte offset kept so a template error can point into the
//! HTML. So is the body of a `## To … -> View:` function (a template macro). The new
//! headers (`## Page /path`, `## Route METHOD /path`, `## Store`, `## Directive`,
//! `## Before each request`, `## Redirect /a -> /b.`, `## Error page 404`) lex to their
//! own `BlockType`s, with paths read raw off the header line (a path is not LOGOS words).
//!
//! The legacy indentation DSL (`View:` followed by `div [class="x"]:` lines) keeps lexing
//! as ordinary tokens until its retirement: its bodies never begin with `<` or `{`.

use logicaffeine_base::Interner;
use logicaffeine_language::token::{BlockType, RawKind, TokenType};
use logicaffeine_language::Lexer;

fn lex(src: &str) -> (Vec<TokenType>, Vec<(TokenType, String, usize, usize)>) {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(src, &mut interner);
    let tokens = lexer.tokenize();
    let kinds: Vec<TokenType> = tokens.iter().map(|t| t.kind.clone()).collect();
    let full: Vec<(TokenType, String, usize, usize)> = tokens
        .iter()
        .map(|t| {
            (
                t.kind.clone(),
                interner.resolve(t.lexeme).to_string(),
                t.span.start,
                t.span.end,
            )
        })
        .collect();
    (kinds, full)
}

fn raw_blocks(src: &str) -> Vec<(RawKind, String, usize)> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(src, &mut interner);
    lexer
        .tokenize()
        .iter()
        .filter_map(|t| match &t.kind {
            TokenType::RawBlock {
                kind,
                body,
                content_start,
                ..
            } => Some((*kind, interner.resolve(*body).to_string(), *content_start)),
            _ => None,
        })
        .collect()
}

fn headers(src: &str) -> Vec<BlockType> {
    lex(src)
        .0
        .into_iter()
        .filter_map(|k| match k {
            TokenType::BlockHeader { block_type } => Some(block_type),
            _ => None,
        })
        .collect()
}

fn resolve(src: &str, sym: logicaffeine_base::Symbol) -> String {
    // Re-lex to recover the interner text for a header's symbol field.
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(src, &mut interner);
    let _ = lexer.tokenize();
    interner.resolve(sym).to_string()
}

const PAGE: &str = "## Page /todos\nState:\n    Let x be 1.\nView:\n    <div class=\"card\">\n      <h1>{x}</h1>\n\n      <p>done</p>\n    </div>\nStyle:\n    .card { color: red; }\n    .card h1 { margin: 0; }\n";

#[test]
fn view_and_style_sections_are_captured_raw_with_inner_indentation_and_blank_lines() {
    let blocks = raw_blocks(PAGE);
    assert_eq!(blocks.len(), 2, "one RawBlock per section: {blocks:?}");
    assert_eq!(blocks[0].0, RawKind::View);
    assert_eq!(
        blocks[0].1,
        "<div class=\"card\">\n  <h1>{x}</h1>\n\n  <p>done</p>\n</div>"
    );
    assert_eq!(blocks[1].0, RawKind::Style);
    assert_eq!(
        blocks[1].1,
        ".card { color: red; }\n.card h1 { margin: 0; }"
    );
}

#[test]
fn the_raw_body_offset_points_at_its_first_byte_in_the_source() {
    let blocks = raw_blocks(PAGE);
    assert_eq!(blocks[0].2, PAGE.find("<div").unwrap(), "View body offset");
    assert_eq!(
        blocks[1].2,
        PAGE.find(".card {").unwrap(),
        "Style body offset"
    );
}

#[test]
fn nothing_inside_a_raw_body_is_tokenized_as_logos() {
    let (kinds, _) = lex(PAGE);
    // The HTML's `<`, `{`, `}` and the CSS braces must never surface as LOGOS tokens.
    let leaked = kinds
        .iter()
        .filter(|k| {
            matches!(
                k,
                TokenType::Lt | TokenType::LBrace | TokenType::RBrace | TokenType::Gt
            )
        })
        .count();
    assert_eq!(
        leaked, 0,
        "raw bodies leaked LOGOS punctuation tokens: {kinds:?}"
    );
    // …and the State section still lexes as ordinary LOGOS.
    assert!(
        kinds.iter().any(|k| matches!(k, TokenType::Let)),
        "the State section still lexes: {kinds:?}"
    );
}

#[test]
fn the_raw_body_is_bracketed_by_indent_and_dedent_like_an_escape_block() {
    let (kinds, _) = lex(PAGE);
    let i = kinds
        .iter()
        .position(|k| {
            matches!(
                k,
                TokenType::RawBlock {
                    kind: RawKind::View,
                    ..
                }
            )
        })
        .expect("View block");
    assert!(
        matches!(kinds[i - 1], TokenType::Indent),
        "Indent precedes the raw body: {:?}",
        &kinds[i.saturating_sub(3)..=i]
    );
    assert!(
        matches!(kinds[i + 1], TokenType::Dedent),
        "Dedent follows the raw body: {:?}",
        &kinds[i..i + 3]
    );
}

#[test]
fn a_legacy_indentation_view_body_is_not_captured_raw() {
    let legacy = "## Component App:\n    Let n be a signal 0.\n    View:\n        h1: \"Count: {n}\"\n        button:\n            on click: Increase n by 1.\n";
    assert!(
        raw_blocks(legacy).is_empty(),
        "a body that starts with an identifier line stays ordinary tokens"
    );
    let (kinds, _) = lex(legacy);
    assert!(
        kinds.iter().any(|k| matches!(
            k,
            TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)
        )),
        "the legacy body still lexes its strings"
    );
}

#[test]
fn a_view_body_starting_with_a_template_tag_is_raw() {
    let src = "## Page /todos\nView:\n    {extends Site}\n    {block content:}\n    <p>hi</p>\n    {end}\n";
    let blocks = raw_blocks(src);
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].1,
        "{extends Site}\n{block content:}\n<p>hi</p>\n{end}"
    );
}

#[test]
fn a_view_returning_function_body_is_raw() {
    let src = "## To badge (label: Text) and (tone: Text) -> View:\n    <span class=\"badge {tone}\">{label | upper}</span>\n\n## Main\nShow 1.\n";
    let blocks = raw_blocks(src);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, RawKind::View);
    assert_eq!(
        blocks[0].1,
        "<span class=\"badge {tone}\">{label | upper}</span>"
    );
    let (kinds, _) = lex(src);
    assert!(
        kinds.iter().any(|k| matches!(k, TokenType::Show)),
        "the program after the raw body still lexes"
    );
}

#[test]
fn a_raw_body_that_reaches_end_of_input_is_captured_whole() {
    let src = "## Page /a\nView:\n    <p>last</p>";
    let blocks = raw_blocks(src);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, "<p>last</p>");
}

#[test]
fn page_and_route_headers_read_their_paths_raw() {
    let src = "## Page /todos/:id (id: Int)\nView:\n    <p>{id}</p>\n";
    let hs = headers(src);
    match hs[0] {
        BlockType::Page { path } => assert_eq!(resolve(src, path), "/todos/:id"),
        other => panic!("expected a Page header, got {other:?}"),
    }
    let (kinds, _) = lex(src);
    assert!(
        kinds.iter().any(|k| matches!(k, TokenType::LParen)),
        "the typed params after the path still lex"
    );

    let src =
        "## Route DELETE /api/todos/:id (id: Int) -> Seq of Todo:\n    Return removeTodo(id).\n";
    match headers(src)[0] {
        BlockType::Route { method, path } => {
            assert_eq!(resolve(src, method), "DELETE");
            assert_eq!(resolve(src, path), "/api/todos/:id");
        }
        other => panic!("expected a Route header, got {other:?}"),
    }
}

#[test]
fn store_directive_middleware_redirect_and_error_page_headers_lex() {
    assert!(matches!(
        headers("## Store Session persisted as \"session\"\nState:\n    Let name be \"guest\".\n")
            [0],
        BlockType::Store
    ));
    assert!(matches!(headers("## Directive tooltip (el: Element) and (text: Text):\n    Set the attribute \"title\" of el to text.\n")[0], BlockType::Directive));
    assert!(matches!(
        headers(
            "## Before each request (request: Request) -> Option of Response:\n    Return none.\n"
        )[0],
        BlockType::Middleware
    ));
    let src = "## Redirect /old-todos -> /todos.\n";
    match headers(src)[0] {
        BlockType::Redirect { from, to } => {
            assert_eq!(resolve(src, from), "/old-todos");
            assert_eq!(resolve(src, to), "/todos");
        }
        other => panic!("expected a Redirect header, got {other:?}"),
    }
    assert!(matches!(
        headers("## Error page 404\nView:\n    <h1>Lost</h1>\n")[0],
        BlockType::ErrorPage { status: 404 }
    ));
}

#[test]
fn a_typo_of_a_new_header_is_loud_not_prose() {
    for typo in [
        "## Pgae /x\n",
        "## Rotue GET /x -> Text:\n",
        "## Stroe S\n",
        "## Directve d (el: Element):\n",
    ] {
        assert!(
            matches!(headers(typo)[0], BlockType::SuspectedTypo { .. }),
            "{typo:?} must be flagged as a probable typo of a consequential header"
        );
    }
}

#[test]
fn prose_headings_that_share_a_first_word_stay_prose() {
    // A literate heading is not a code header unless the LINE has the LGX shape.
    for prose in [
        "## Error Propagation\nSome prose.\n",
        "## Before you begin\nRead this.\n",
    ] {
        assert!(
            matches!(headers(prose)[0], BlockType::Note),
            "{prose:?} must stay literate prose"
        );
    }
}

#[test]
fn a_code_header_word_without_its_shape_is_loud() {
    // `Page`, `Route`, `Store`, `Directive`, `Redirect` are consequential: a malformed one is
    // reported, never silently demoted to prose.
    for bad in [
        "## Page layout\n",
        "## Store\n",
        "## Route todos\n",
        "## Redirect /old\n",
    ] {
        assert!(
            matches!(headers(bad)[0], BlockType::SuspectedTypo { .. }),
            "{bad:?} must be flagged, got {:?}",
            headers(bad)[0]
        );
    }
}
