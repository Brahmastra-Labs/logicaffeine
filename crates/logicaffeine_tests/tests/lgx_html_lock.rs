//! ★ LOCK — the LGX HTML tokenizer: real markup, `{…}` holes, entities, void elements,
//! spans that point into the input, and Socratic errors. ★
//!
//! The tokenizer is FLAT and pure: it turns a raw `View:` body into start/end tags, text,
//! and brace groups without building a tree (nesting, whitespace collapse and template
//! tags are `lgx::template`'s job). Brace scanning is string-aware and nesting-aware, so a
//! LOGOS interpolation inside a hole (`{greet("{x}")}`) stays one group. Every token and
//! attribute carries the byte span of its source so an error can point into the HTML.

use logicaffeine_language::lgx::html::{tokenize, AttrPart, AttrValue, HtmlErrorKind, HtmlToken};

fn toks(src: &str) -> Vec<HtmlToken> {
    tokenize(src).unwrap_or_else(|e| panic!("{src:?} must tokenize: {e:?}"))
}

fn shape(tokens: &[HtmlToken]) -> Vec<String> {
    tokens
        .iter()
        .map(|t| match t {
            HtmlToken::StartTag {
                name,
                attrs,
                self_closing,
                ..
            } => {
                let a: Vec<String> = attrs
                    .iter()
                    .map(|a| match &a.value {
                        None => a.name.clone(),
                        Some(AttrValue::Static(s)) => format!("{}={s:?}", a.name),
                        Some(AttrValue::Parts(parts)) => {
                            let ps: Vec<String> = parts
                                .iter()
                                .map(|p| match p {
                                    AttrPart::Text(s) => format!("{s:?}"),
                                    AttrPart::Brace { inner, .. } => format!("{{{inner}}}"),
                                })
                                .collect();
                            format!("{}=[{}]", a.name, ps.join(" "))
                        }
                    })
                    .collect();
                format!(
                    "<{}{}{}>",
                    name,
                    if a.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", a.join(" "))
                    },
                    if *self_closing { "/" } else { "" }
                )
            }
            HtmlToken::EndTag { name, .. } => format!("</{name}>"),
            HtmlToken::Text { text, .. } => format!("{text:?}"),
            HtmlToken::Brace { inner, .. } => format!("{{{inner}}}"),
        })
        .collect()
}

#[test]
fn an_element_with_text() {
    assert_eq!(
        shape(&toks("<h1>Hello</h1>")),
        ["<h1>", "\"Hello\"", "</h1>"]
    );
}

#[test]
fn quoted_unquoted_and_valueless_attributes() {
    assert_eq!(
        shape(&toks("<input type=\"text\" id=draft disabled>")),
        ["<input type=\"text\" id=\"draft\" disabled/>"]
    );
    assert_eq!(
        shape(&toks("<a href='/x'>go</a>")),
        ["<a href=\"/x\">", "\"go\"", "</a>"]
    );
}

#[test]
fn dynamic_attribute_values_split_into_parts() {
    assert_eq!(
        shape(&toks("<span class=\"badge {tone}\" title={t}>x</span>")),
        [
            "<span class=[\"badge \" {tone}] title=[{t}]>",
            "\"x\"",
            "</span>"
        ]
    );
}

#[test]
fn holes_in_text_are_brace_groups() {
    assert_eq!(
        shape(&toks("<p>n = {count} left</p>")),
        ["<p>", "\"n = \"", "{count}", "\" left\"", "</p>"]
    );
}

#[test]
fn braces_are_string_and_nesting_aware() {
    assert_eq!(
        shape(&toks("<p>{greet(\"{x}\", 1)}</p>")),
        ["<p>", "{greet(\"{x}\", 1)}", "</p>"]
    );
    assert_eq!(
        shape(&toks("<p>{if a is \"}\":}yes{end}</p>")),
        ["<p>", "{if a is \"}\":}", "\"yes\"", "{end}", "</p>"]
    );
}

#[test]
fn entities_decode_in_text_and_static_attribute_values() {
    assert_eq!(
        shape(&toks(
            "<p title=\"a &amp; b\">&lt;tag&gt; &#169; &#x41; &quot;q&quot;</p>"
        )),
        ["<p title=\"a & b\">", "\"<tag> © A \\\"q\\\"\"", "</p>"]
    );
}

#[test]
fn void_elements_and_explicit_self_closing_tags_never_expect_an_end_tag() {
    assert_eq!(
        shape(&toks("<br><img src=\"x.png\"><TodoRow todo={t}/>")),
        ["<br/>", "<img src=\"x.png\"/>", "<TodoRow todo=[{t}]/>"]
    );
}

#[test]
fn component_names_keep_their_case() {
    assert_eq!(
        shape(&toks("<TodoRow/><div></div>")),
        ["<TodoRow/>", "<div>", "</div>"]
    );
}

#[test]
fn comments_are_dropped() {
    assert_eq!(
        shape(&toks("<p><!-- a note -->x<!--\n multi\n--></p>")),
        ["<p>", "\"x\"", "</p>"]
    );
}

#[test]
fn whitespace_is_kept_verbatim_by_the_tokenizer() {
    // Collapsing is the tree builder's decision (it knows about <pre>); the tokenizer keeps bytes.
    assert_eq!(
        shape(&toks("<pre>  a\n   b</pre>")),
        ["<pre>", "\"  a\\n   b\"", "</pre>"]
    );
    assert_eq!(
        shape(&toks("<ul>\n  <li>x</li>\n</ul>")),
        [
            "<ul>",
            "\"\\n  \"",
            "<li>",
            "\"x\"",
            "</li>",
            "\"\\n\"",
            "</ul>"
        ]
    );
}

#[test]
fn spans_point_into_the_input() {
    let src = "<div class=\"c\">{count}</div>";
    let t = toks(src);
    match &t[0] {
        HtmlToken::StartTag { span, attrs, .. } => {
            assert_eq!((span.start, span.end), (0, 15));
            assert_eq!(&src[attrs[0].span.start..attrs[0].span.end], "class=\"c\"");
        }
        other => panic!("{other:?}"),
    }
    match &t[1] {
        HtmlToken::Brace { span, .. } => assert_eq!(&src[span.start..span.end], "{count}"),
        other => panic!("{other:?}"),
    }
    match &t[2] {
        HtmlToken::EndTag { span, .. } => assert_eq!(&src[span.start..span.end], "</div>"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn script_style_and_doctype_are_forbidden_in_a_view() {
    for (src, name) in [
        ("<script>alert(1)</script>", "script"),
        ("<style>.a{}</style>", "style"),
        ("<!doctype html><p>x</p>", "doctype"),
    ] {
        let err = tokenize(src).expect_err("must be refused");
        assert_eq!(
            err.kind,
            HtmlErrorKind::ForbiddenElement(name.to_string()),
            "{src}"
        );
        assert!(!err.message().is_empty(), "the error explains itself");
    }
}

#[test]
fn unterminated_constructs_are_precise_errors() {
    let e = tokenize("<div class=\"x\"").expect_err("unterminated tag");
    assert_eq!(e.kind, HtmlErrorKind::UnterminatedTag);
    let e = tokenize("<p>{count</p>").expect_err("unterminated brace");
    assert_eq!(e.kind, HtmlErrorKind::UnterminatedBrace);
    assert_eq!(e.span.start, 3, "the error points at the opening brace");
    let e = tokenize("<p title=\"x>y</p>").expect_err("unterminated attribute value");
    assert_eq!(e.kind, HtmlErrorKind::UnterminatedAttributeValue);
    let e = tokenize("<p =x>").expect_err("attribute needs a name");
    assert!(matches!(e.kind, HtmlErrorKind::BadAttributeName(_)));
}

#[test]
fn hostile_input_never_panics() {
    let corpus = [
        "",
        "<",
        ">",
        "<>",
        "</",
        "</>",
        "<a",
        "<a b",
        "<a b=",
        "<a b=\"",
        "<a b='",
        "{",
        "}",
        "{{",
        "}}",
        "<p>{\"unterminated</p>",
        "<!--",
        "<!-- -->",
        "<a/",
        "<a//>",
        "<1>",
        "< a>",
        "<a b c d>",
        "&",
        "&#;",
        "&#xZZ;",
        "<p>&amp</p>",
        "\u{0}",
        "<p>\u{FFFD}</p>",
        "<a\n\tb\n=\n\"c\">",
        "{{{{{{",
        "</a></b></c>",
        "<TodoRow/></TodoRow>",
    ];
    for src in corpus {
        let _ = tokenize(src);
    }
}
