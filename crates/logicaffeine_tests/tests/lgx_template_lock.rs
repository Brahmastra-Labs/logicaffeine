//! ★ LOCK — the LGX template parser: HTML tokens + Jinja-style `{…}` tags → a template
//! tree with every LOGOS expression kept as TEXT plus its byte offset. ★
//!
//! `lgx::template::parse` is pure and knows nothing about the LOGOS expression grammar:
//! holes, conditions, iterables and filter arguments come out as `TExpr { text, offset }`
//! for the parser to sub-lex with the real expression parser (so an error inside a hole
//! points at the hole). It owns nesting (`{end}` matching, element close tags), the
//! whitespace rules (whitespace-only text between tags is dropped, runs collapse to one
//! space, `<pre>`/`<textarea>` verbatim), directives (`on-<event>.<mods>`, `bind`,
//! `class:name`), component tags (Capitalized), filters (`{x | f | g "arg"}`), and the
//! Socratic errors for every structural mistake.

use logicaffeine_language::lgx::html::AttrPart;
use logicaffeine_language::lgx::template::{parse, TAttr, TExpr, TNode, TemplateErrorKind};

fn expr(e: &TExpr) -> String {
    e.text.trim().to_string()
}

fn attrs(a: &[TAttr]) -> String {
    let items: Vec<String> = a
        .iter()
        .map(|a| match a {
            TAttr::Static {
                name, value: None, ..
            } => name.clone(),
            TAttr::Static {
                name,
                value: Some(v),
                ..
            } => format!("{name}={v:?}"),
            TAttr::Dynamic { name, parts, .. } => {
                let ps: Vec<String> = parts
                    .iter()
                    .map(|p| match p {
                        AttrPart::Text(t) => format!("{t:?}"),
                        AttrPart::Brace { inner, .. } => format!("{{{}}}", inner.trim()),
                    })
                    .collect();
                format!("{name}=[{}]", ps.join(" "))
            }
            TAttr::On {
                event,
                modifiers,
                expr: e,
                ..
            } => {
                let words: Vec<String> = modifiers.iter().map(|m| m.word()).collect();
                let mods = if words.is_empty() {
                    String::new()
                } else {
                    format!(".{}", words.join("."))
                };
                format!("on:{event}{mods}={{{}}}", expr(e))
            }
            TAttr::Bind { field, .. } => format!("bind={field}"),
            TAttr::Class { name, expr: e, .. } => format!("class:{name}={{{}}}", expr(e)),
        })
        .collect();
    if items.is_empty() {
        String::new()
    } else {
        format!("[{}]", items.join(", "))
    }
}

fn body(nodes: &[TNode]) -> String {
    let inner: Vec<String> = nodes.iter().map(shape).collect();
    format!("{{{}}}", inner.join(", "))
}

fn shape(n: &TNode) -> String {
    match n {
        TNode::Element {
            tag,
            attrs: a,
            children,
            ..
        } => {
            format!(
                "{tag}{}{}",
                attrs(a),
                if children.is_empty() {
                    String::new()
                } else {
                    body(children)
                }
            )
        }
        TNode::Text { text, .. } => format!("{text:?}"),
        TNode::Hole {
            expr: e,
            filters,
            raw,
            ..
        } => {
            let fs: Vec<String> = filters
                .iter()
                .map(|f| {
                    if f.args.is_empty() {
                        f.name.clone()
                    } else {
                        format!(
                            "{}({})",
                            f.name,
                            f.args.iter().map(expr).collect::<Vec<_>>().join(", ")
                        )
                    }
                })
                .collect();
            let fs = if fs.is_empty() {
                String::new()
            } else {
                format!(" |{}", fs.join("|"))
            };
            format!("{}({}{})", if *raw { "raw" } else { "hole" }, expr(e), fs)
        }
        TNode::If {
            arms, otherwise, ..
        } => {
            let mut s = String::new();
            for (i, (c, b)) in arms.iter().enumerate() {
                s.push_str(&format!(
                    "{}({}){}",
                    if i == 0 { "if" } else { " elif" },
                    expr(c),
                    body(b)
                ));
            }
            if let Some(o) = otherwise {
                s.push_str(&format!(" else{}", body(o)));
            }
            s
        }
        TNode::For {
            pattern,
            iterable,
            key,
            index,
            body: b,
            empty,
            ..
        } => {
            let mut s = format!("each({pattern} in {}", expr(iterable));
            if let Some(k) = key {
                s.push_str(&format!("; key={}", expr(k)));
            }
            if let Some(i) = index {
                s.push_str(&format!("; index={i}"));
            }
            s.push(')');
            s.push_str(&body(b));
            if let Some(e) = empty {
                s.push_str(&format!(" else{}", body(e)));
            }
            s
        }
        TNode::Inspect {
            scrutinee, arms, ..
        } => {
            let a: Vec<String> = arms
                .iter()
                .map(|(p, b)| format!("when({}){}", p.trim(), body(b)))
                .collect();
            format!("inspect({}){{{}}}", expr(scrutinee), a.join(" "))
        }
        TNode::Let { name, expr: e, .. } => format!("let({name} = {})", expr(e)),
        TNode::Extends { name, .. } => format!("extends({name})"),
        TNode::Block { name, default, .. } => match default {
            None => format!("block({name})"),
            Some(d) => format!("block({name}){}", body(d)),
        },
        TNode::Children { .. } => "children".to_string(),
        TNode::Component {
            name,
            props,
            children,
            ..
        } => {
            format!(
                "<{name}{}>{}",
                attrs(props),
                if children.is_empty() {
                    String::new()
                } else {
                    body(children)
                }
            )
        }
    }
}

fn parsed(src: &str) -> String {
    let nodes = parse(src).unwrap_or_else(|e| panic!("{src:?} must parse: {e:?}"));
    nodes.iter().map(shape).collect::<Vec<_>>().join(" ")
}

fn error_kind(src: &str) -> TemplateErrorKind {
    parse(src)
        .expect_err(&format!("{src:?} must be refused"))
        .kind
}

#[test]
fn text_and_holes_with_filters() {
    assert_eq!(
        parsed("<p>Hi {name | upper}!</p>"),
        "p{\"Hi \", hole(name |upper), \"!\"}"
    );
    assert_eq!(
        parsed("<p>{price | money | pad 8}</p>"),
        "p{hole(price |money|pad(8))}"
    );
    assert_eq!(
        parsed("<p>{name | default \"anon\"}</p>"),
        "p{hole(name |default(\"anon\"))}"
    );
    assert_eq!(parsed("<p>{a | b}</p>"), "p{hole(a |b)}");
}

#[test]
fn a_pipe_inside_a_string_or_parens_is_not_a_filter() {
    assert_eq!(
        parsed("<p>{join(xs, \" | \")}</p>"),
        "p{hole(join(xs, \" | \"))}"
    );
    assert_eq!(parsed("<p>{(a | b)}</p>"), "p{hole((a | b))}");
}

#[test]
fn if_else_if_else_chains() {
    assert_eq!(
        parsed("{if a:}<b>1</b>{else if c:}<i>2</i>{else:}3{end}"),
        "if(a){b{\"1\"}} elif(c){i{\"2\"}} else{\"3\"}"
    );
    assert_eq!(parsed("<p>{if ok:}yes{end}</p>"), "p{if(ok){\"yes\"}}");
}

#[test]
fn for_each_with_key_index_and_empty_arm() {
    assert_eq!(
        parsed("{for each todo in todos, keyed by todo's id, at index i:}<li>{todo's text}</li>{else:}<li>none</li>{end}"),
        "each(todo in todos; key=todo's id; index=i){li{hole(todo's text)}} else{li{\"none\"}}"
    );
    assert_eq!(
        parsed("{for each x in xs:}<i>{x}</i>{end}"),
        "each(x in xs){i{hole(x)}}"
    );
    assert_eq!(
        parsed("{for each (k, v) in m:}<i>{k}</i>{end}"),
        "each((k, v) in m){i{hole(k)}}"
    );
}

#[test]
fn inspect_with_when_arms() {
    assert_eq!(
        parsed("{inspect r:}{when Ok (v):}<p>{v}</p>{when Err (e):}<p>{e}</p>{end}"),
        "inspect(r){when(Ok (v)){p{hole(v)}} when(Err (e)){p{hole(e)}}}"
    );
}

#[test]
fn let_extends_block_children_raw_and_comments() {
    assert_eq!(
        parsed("{let total be sum(prices)}<p>{total}</p>"),
        "let(total = sum(prices)) p{hole(total)}"
    );
    assert_eq!(
        parsed("{extends Site}{block title:}Todos{end}{block content:}<p>x</p>{end}"),
        "extends(Site) block(title){\"Todos\"} block(content){p{\"x\"}}"
    );
    assert_eq!(
        parsed("<main>{block content}</main>"),
        "main{block(content)}"
    );
    assert_eq!(
        parsed("<div>{children}{raw html}</div>"),
        "div{children, raw(html)}"
    );
    assert_eq!(parsed("<p>{# a note #}x</p>"), "p{\"x\"}");
}

#[test]
fn component_tags_carry_props_and_children() {
    assert_eq!(
        parsed("<Card title=\"Todos\" count={n}><p>body</p></Card>"),
        "<Card[title=\"Todos\", count=[{n}]]>{p{\"body\"}}"
    );
    assert_eq!(
        parsed("<TodoRow todo={t} remove={remove}/>"),
        "<TodoRow[todo=[{t}], remove=[{remove}]]>"
    );
}

#[test]
fn directives_are_recognized_on_elements() {
    assert_eq!(
        parsed("<form on-submit.prevent={add}><input bind={draft} class:done={ok} tooltip={t} disabled={busy}></form>"),
        "form[on:submit.prevent={add}]{input[bind=draft, class:done={ok}, tooltip=[{t}], disabled=[{busy}]]}"
    );
    assert_eq!(
        parsed("<button on-click={remove(todo's id)}>x</button>"),
        "button[on:click={remove(todo's id)}]{\"x\"}"
    );
    assert_eq!(
        parsed("<input on-keydown.enter.prevent={add}>"),
        "input[on:keydown.enter.prevent={add}]"
    );
}

#[test]
fn whitespace_between_tags_is_dropped_runs_collapse_and_pre_is_verbatim() {
    assert_eq!(
        parsed("<ul>\n  <li>a</li>\n  <li>b</li>\n</ul>"),
        "ul{li{\"a\"}, li{\"b\"}}"
    );
    assert_eq!(parsed("<p>a\n  b   c</p>"), "p{\"a b c\"}");
    assert_eq!(parsed("<p> {x} </p>"), "p{\" \", hole(x), \" \"}");
    assert_eq!(parsed("<pre>  a\n   b</pre>"), "pre{\"  a\\n   b\"}");
    assert_eq!(parsed("<textarea>\n x</textarea>"), "textarea{\"\\n x\"}");
}

#[test]
fn every_node_and_hole_carries_its_span() {
    let src = "<p>{count}</p>";
    let nodes = parse(src).unwrap();
    match &nodes[0] {
        TNode::Element { span, children, .. } => {
            assert_eq!((span.start, span.end), (0, src.len()));
            match &children[0] {
                TNode::Hole { expr, span, .. } => {
                    assert_eq!(&src[span.start..span.end], "{count}");
                    assert_eq!(expr.offset, 4, "the expression offset points at `count`");
                    assert_eq!(&src[expr.offset..expr.offset + expr.text.len()], "count");
                }
                other => panic!("{other:?}"),
            }
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn structural_mistakes_are_precise_socratic_errors() {
    assert!(matches!(
        error_kind("<p>{end}</p>"),
        TemplateErrorKind::UnexpectedEnd
    ));
    assert!(
        matches!(error_kind("{if a:}<p>x</p>"), TemplateErrorKind::UnclosedTag(ref t) if t == "if")
    );
    assert!(
        matches!(error_kind("<div><span></div>"), TemplateErrorKind::MismatchedEndTag { ref expected, ref found } if expected == "span" && found == "div")
    );
    assert!(
        matches!(error_kind("<div><p>x</p>"), TemplateErrorKind::UnclosedElement(ref t) if t == "div")
    );
    assert!(matches!(error_kind("</p>"), TemplateErrorKind::StrayEndTag(ref t) if t == "p"));
    assert!(matches!(
        error_kind("<p>x</p>{extends Site}"),
        TemplateErrorKind::ExtendsNotFirst
    ));
    assert!(matches!(
        error_kind("<p>{else:}x</p>"),
        TemplateErrorKind::StrayElse
    ));
    assert!(matches!(
        error_kind("{fro each x in xs:}<i>{x}</i>{end}"),
        TemplateErrorKind::TagLikeHole(_)
    ));
    assert!(matches!(
        error_kind("<p>{if a:}{else:}{else:}{end}</p>"),
        TemplateErrorKind::DuplicateElse
    ));
    assert!(
        matches!(error_kind("<input bind={a}>{bind}</input>"), TemplateErrorKind::UnexpectedEndTagOfVoid(ref t) if t == "input")
    );
    assert!(matches!(
        error_kind("<script>x</script>"),
        TemplateErrorKind::Html(_)
    ));
    // Wave 4: another template language's tag words are refused with the LGX spelling, not
    // read as a hole naming a variable called `elif`/`otherwise`/`endif` — `TAG_ALIASES`'
    // own census in `lgx/template.rs`.
    assert!(matches!(
        error_kind("{if a:}x{elif b:}y{end}"),
        TemplateErrorKind::NonCanonicalTag { ref written, ref canonical }
            if written == "{elif b:}" && canonical == "{else if b:}"
    ));
    assert!(matches!(
        error_kind("{if a:}x{otherwise:}y{end}"),
        TemplateErrorKind::NonCanonicalTag { ref written, ref canonical }
            if written == "{otherwise:}" && canonical == "{else:}"
    ));
    assert!(matches!(
        error_kind("{if a:}x{end}{endif}"),
        TemplateErrorKind::NonCanonicalTag { ref written, ref canonical }
            if written == "{endif}" && canonical == "{end}"
    ));
    let elif_err = parse("{if a:}x{elif b:}y{end}").unwrap_err();
    assert!(
        elif_err.message().contains("{else if c:}") || elif_err.message().contains("else if"),
        "the refusal teaches the canonical `{{else if c:}}` spelling: {}",
        elif_err.message()
    );
    let e = parse("{if a:}<p>x</p>").unwrap_err();
    assert_eq!(
        e.span.start, 0,
        "an unclosed tag's error points at its opener"
    );
    assert!(
        e.message().contains("{end}"),
        "the message names the fix: {}",
        e.message()
    );
}

/// ★ THE RETIRED `Render:` LINES ARE REFUSED, SHAPE-EXACT. A line that is the old dialect's
/// element (`div: card("Hello")`, `p: "hi"`, `li:`, `div [class="x"]:`) or its stylesheet
/// (`css "…"`) would otherwise become literal text on the page, or — with a `{` in its CSS — a
/// hole refused for the wrong reason. Only the whole line in exactly those shapes is one: prose that
/// mentions a tag word, and anything inside `<pre>`, is text as written.
#[test]
fn a_retired_render_line_is_refused_and_prose_is_not() {
    use logicaffeine_language::lgx::template::RetiredForm;
    let element =
        |tag: &str| TemplateErrorKind::RetiredViewForm(RetiredForm::Element(tag.to_string()));
    let stylesheet = TemplateErrorKind::RetiredViewForm(RetiredForm::Stylesheet);
    for (src, kind) in [
        ("<div>\n  div: card(\"Hello\")\n</div>", element("div")),
        ("<ul>\n  li:\n</ul>", element("li")),
        ("<div>\n  p: \"hi\"\n</div>", element("p")),
        ("<main>\n  div [class=\"x\"]:\n</main>", element("div")),
        ("div: card(\"Hello\")\n<p>x</p>", element("div")),
        (
            "<div>\n  css \"p { color: red }\"\n</div>",
            stylesheet.clone(),
        ),
        (
            "<div>\n  css r\"p { color: red }\"\n</div>",
            stylesheet.clone(),
        ),
    ] {
        assert_eq!(error_kind(src), kind, "{src:?}");
    }
    let e = parse("<div>\n  div: card(\"Hello\")\n</div>").unwrap_err();
    assert_eq!(
        e.span.start,
        "<div>\n  ".len(),
        "the refusal points at the line's first word"
    );
    assert!(
        e.message().contains("<div>") && e.message().contains("<Card"),
        "{}",
        e.message()
    );
    let e = parse("<div>\n  css \"p { color: red }\"\n</div>").unwrap_err();
    assert!(e.message().contains("`Style:`"), "{}", e.message());
    for prose in [
        "<p>\n  a: first option\n</p>",
        "<p>\n  Note: see below\n</p>",
        "<p>\n  div: the element\n</p>",
        "<p>\n  time: {n}\n</p>",
        "<p>css is fun</p>",
        "<p>div: card(\"Hello\")</p>",
        "<pre>\n  div: card(\"Hello\")\n</pre>",
        "<pre>\n  css \"p { color: red }\"\n</pre>",
    ] {
        assert!(
            parse(prose).is_ok(),
            "{prose:?} is text as written: {:?}",
            parse(prose).err()
        );
    }
}

#[test]
fn hostile_templates_never_panic() {
    let corpus = [
        "",
        "{",
        "}",
        "{end}",
        "{if}",
        "{if :}",
        "{for each}",
        "{for each x:}",
        "{block}",
        "{extends}",
        "{let x}",
        "{when x:}",
        "<a>{if a:}</a>{end}",
        "{if a:}<a>{end}</a>",
        "{{}}",
        "{|}",
        "{x |}",
        "{| f}",
        "<Comp",
        "<Comp a={",
        "{# unterminated",
        "<p>{raw}</p>",
        "<p>{children extra}</p>",
        "{for each x in xs, keyed by:}{end}",
    ];
    for src in corpus {
        let _ = parse(src);
    }
}
