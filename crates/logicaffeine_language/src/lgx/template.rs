//! The LGX template parser — HTML tokens plus Jinja-style `{…}` tags into a template tree.
//!
//! Pure and grammar-agnostic about LOGOS: every expression (a hole, a condition, an
//! iterable, a filter argument) comes out as [`TExpr`] — its text and byte offset — for the
//! LOGOS parser to sub-lex with the real expression parser, so a mistake inside a hole is
//! reported at the hole. This module owns everything structural instead:
//!
//! - nesting: elements close with their end tag, tags close with `{end}`, and the two may
//!   not cross (`{if a:}<p>{end}</p>` is an error at the `{end}`);
//! - the tags: `{if c:}`/`{else if c:}`/`{else:}`, `{for each x in xs[, keyed by k][, at index
//!   i]:}` with an `{else:}` arm for an empty collection, `{inspect v:}`/`{when Pat:}`,
//!   `{let x be e}`, `{extends Layout}` (first, or nowhere), `{block name:}…{end}` /
//!   `{block name}`, `{children}`, `{raw e}`, `{# a comment #}`, and `{end}`;
//! - holes with filters: `{price | money | pad 8}` — a filter is a function name followed by
//!   its extra arguments, split at the top level (a `|` inside a string or parens is not a
//!   filter);
//! - components: a Capitalized tag is a component use, its attributes are props;
//! - directives: `on-<event>[.mod…]={…}`, `bind={field}`, `class:name={…}`;
//! - whitespace: text that is only whitespace AND contains a newline is formatting and is
//!   dropped; other whitespace runs collapse to one space; `<pre>`/`<textarea>` are verbatim.
//!
//! A misspelled tag word can only look like a hole, so a hole whose text ends in `:` is
//! reported as a tag-like hole with the likely fix.

use super::html::{
    self, AttrPart, AttrValue, EventModifier, HtmlAttr, HtmlErrorKind, HtmlToken, ModifierError,
};
use crate::token::Span;

/// A LOGOS expression kept as text, with the byte offset of `text` in the template source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TExpr {
    pub text: String,
    pub offset: usize,
}

/// One filter application: `| name arg arg…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TFilter {
    pub name: String,
    pub args: Vec<TExpr>,
}

/// An attribute or directive on an element / a prop on a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TAttr {
    Static {
        name: String,
        value: Option<String>,
        span: Span,
    },
    Dynamic {
        name: String,
        parts: Vec<AttrPart>,
        span: Span,
    },
    /// `on-<event>[.<qualifier>…]={…}`, its qualifiers classified by the event-modifier catalogue.
    On {
        event: String,
        modifiers: Vec<EventModifier>,
        expr: TExpr,
        span: Span,
    },
    /// `bind={field}`; `at` is the byte offset of `field` in the template source.
    Bind {
        field: String,
        at: usize,
        span: Span,
    },
    Class {
        name: String,
        expr: TExpr,
        span: Span,
    },
}

/// One node of the template tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TNode {
    Element {
        tag: String,
        attrs: Vec<TAttr>,
        children: Vec<TNode>,
        span: Span,
    },
    Text {
        text: String,
        span: Span,
    },
    Hole {
        expr: TExpr,
        filters: Vec<TFilter>,
        raw: bool,
        span: Span,
    },
    If {
        arms: Vec<(TExpr, Vec<TNode>)>,
        otherwise: Option<Vec<TNode>>,
        span: Span,
    },
    For {
        pattern: String,
        iterable: TExpr,
        key: Option<TExpr>,
        index: Option<String>,
        body: Vec<TNode>,
        empty: Option<Vec<TNode>>,
        span: Span,
    },
    Inspect {
        scrutinee: TExpr,
        arms: Vec<(String, Vec<TNode>)>,
        span: Span,
    },
    Let {
        name: String,
        expr: TExpr,
        span: Span,
    },
    Extends {
        name: String,
        span: Span,
    },
    Block {
        name: String,
        default: Option<Vec<TNode>>,
        span: Span,
    },
    Children {
        span: Span,
    },
    Component {
        name: String,
        props: Vec<TAttr>,
        children: Vec<TNode>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateErrorKind {
    Html(HtmlErrorKind),
    /// `{end}` with nothing open.
    UnexpectedEnd,
    /// A template tag (`if`, `for each`, `inspect`, `block`) still open at the end, or closed
    /// by an element's end tag.
    UnclosedTag(String),
    /// `</found>` where `<expected>` is the innermost open element.
    MismatchedEndTag {
        expected: String,
        found: String,
    },
    /// An element still open at the end of the template, or at an `{end}`.
    UnclosedElement(String),
    /// `</name>` with no open element.
    StrayEndTag(String),
    /// A void element (`<input>`) given an end tag.
    UnexpectedEndTagOfVoid(String),
    /// `{extends …}` anywhere but first.
    ExtendsNotFirst,
    /// `{else:}` / `{else if …:}` outside an `if` (or `else:` outside `if`/`for each`).
    StrayElse,
    /// A second `{else:}` in the same `if` / `for each`.
    DuplicateElse,
    /// `{when …:}` outside an `{inspect …:}`.
    StrayWhen,
    /// A hole whose text ends in `:` — almost certainly a misspelled tag.
    TagLikeHole(String),
    /// A tag that is missing a part (`{if:}`, `{for each x:}`, `{let x}`, `{block}`).
    MalformedTag(String),
    /// `{}` or `{   }`.
    EmptyHole,
    /// A directive with the wrong value shape (`on-click="add"`, `bind="x"`).
    MalformedDirective(String),
    /// A tag spelled the way another template language spells it (`{elif c:}`, `{otherwise:}`,
    /// `{endif}`), with the LGX spelling: `(written, canonical)`.
    NonCanonicalTag {
        written: String,
        canonical: String,
    },
    /// An `on-<event>.<qualifier>` whose qualifier the event-modifier catalogue refuses.
    EventModifier(ModifierError),
    /// A line written in the retired `Render:` dialect ([`retired_view_form`]).
    RetiredViewForm(RetiredForm),
}

/// A line of the retired `Render:` dialect, which a view no longer reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetiredForm {
    /// An element named by its tag and a colon: `div:`, `p: "hi"`, `div: card("Hello")`,
    /// `div [class="x"]:`.
    Element(String),
    /// A stylesheet written into the view: `css "…"` or `css r"…"`.
    Stylesheet,
}

/// The retired form `line` is written in, read from the WHOLE line and only in the dialect's exact
/// shapes, so text that merely mentions a tag word (`a: first option`, `Note: see below`) is not
/// one: an HTML element name, optionally a bracketed attribute list, then `:` and nothing, a string
/// literal, or a call; or `css` followed by a string literal.
pub fn retired_view_form(line: &str) -> Option<RetiredForm> {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("css") {
        if rest.starts_with(char::is_whitespace) {
            let rest = rest.trim_start();
            if rest.starts_with('"') || rest.starts_with("r\"") {
                return Some(RetiredForm::Stylesheet);
            }
        }
    }
    let tag_end = t
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .unwrap_or(t.len());
    let tag = &t[..tag_end];
    if tag.is_empty() || !html::is_html_element(tag) {
        return None;
    }
    let mut rest = &t[tag_end..];
    if let Some(attrs) = rest.trim_start().strip_prefix('[') {
        rest = &attrs[attrs.find(']')? + 1..];
    }
    let value = rest.trim_start().strip_prefix(':')?.trim();
    let is_call = value.ends_with(')')
        && value.split_once('(').is_some_and(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    (value.is_empty() || value.starts_with('"') || is_call)
        .then(|| RetiredForm::Element(tag.to_string()))
}

/// The first line that STARTS inside `span` of `src` and is written in a retired form, with the
/// span of its words. The line is read whole from `src`, past the end of `span` when a tag or a
/// brace ends the text there.
fn retired_line_in(src: &str, span: Span) -> Option<(RetiredForm, Span)> {
    (span.start..span.end)
        .filter(|&at| at == 0 || src.as_bytes()[at - 1] == b'\n')
        .find_map(|start| {
            let end = src[start..].find('\n').map_or(src.len(), |n| start + n);
            let line = &src[start..end];
            let form = retired_view_form(line)?;
            let words = start + (line.len() - line.trim_start().len());
            Some((form, Span::new(words, start + line.trim_end().len())))
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    pub kind: TemplateErrorKind,
    pub span: Span,
}

impl TemplateError {
    /// The Socratic explanation: what went wrong and what to write instead.
    pub fn message(&self) -> String {
        use TemplateErrorKind::*;
        match &self.kind {
            Html(k) => html::HtmlError { kind: k.clone(), span: self.span }.message(),
            UnexpectedEnd => "This `{end}` closes nothing — there is no open `{if …:}`, `{for each …:}`, `{inspect …:}` or `{block …:}` here.".into(),
            UnclosedTag(t) => format!("This `{{{t} …:}}` is never closed. Every template tag that opens a body ends with `{{end}}` — and it must close before the element that contains it does."),
            MismatchedEndTag { expected, found } => format!("`</{found}>` closes the wrong element: the innermost open element is `<{expected}>`. Close `<{expected}>` first, or fix the tag name."),
            UnclosedElement(t) => format!("`<{t}>` is never closed. Add `</{t}>` (or write it as `<{t} …/>` if it has no children)."),
            StrayEndTag(t) => format!("`</{t}>` closes nothing — no `<{t}>` is open here."),
            UnexpectedEndTagOfVoid(t) => format!("`<{t}>` is a void element: it never has children or an end tag. Remove `</{t}>`."),
            ExtendsNotFirst => "`{extends …}` must be the very first thing in a view: the layout is chosen before any content.".into(),
            StrayElse => "This `{else…}` has no `{if …:}` (or `{for each …:}`) to belong to — it must sit between the opener and its `{end}`.".into(),
            DuplicateElse => "An `{if …:}` or `{for each …:}` can have only one `{else:}` arm.".into(),
            StrayWhen => "`{when …:}` only belongs inside `{inspect …:} … {end}`.".into(),
            TagLikeHole(t) => format!("`{{{t}}}` ends in `:` like a template tag, but `{}` is not one. The tags are `if`, `else if`, `else`, `for each`, `inspect`, `when`, `let`, `extends`, `block`, `children`, `raw`, `end` — or drop the `:` to make it a hole.", t.split_whitespace().next().unwrap_or("")),
            MalformedTag(t) => format!("This `{{{t} …}}` tag is missing a part. Shapes: `{{if condition:}}`, `{{else if condition:}}`, `{{else:}}`, `{{for each item in items[, keyed by key][, at index i]:}}`, `{{inspect value:}}`, `{{when Pattern:}}`, `{{let name be expression}}`, `{{extends Layout}}`, `{{block name:}}` / `{{block name}}`, `{{children}}`, `{{raw expression}}`, `{{end}}`."),
            EmptyHole => "This `{}` is empty. A hole holds a LOGOS expression: `{name}`, `{length of todos}`, `{price | money}`.".into(),
            MalformedDirective(d) => format!("`{d}` needs a `{{…}}` value: `on-click={{add}}`, `on-click={{remove(todo's id)}}`, `bind={{draft}}` (a state field), `class:done={{todo's done}}`."),
            EventModifier(e) => e.message(),
            NonCanonicalTag { written, canonical } => format!(
                "`{written}` is written `{canonical}` in an LGX template. The tags are `{{if c:}}`, \
                 `{{else if c:}}`, `{{else:}}`, `{{for each x in xs:}}`, `{{inspect v:}}`, \
                 `{{when P:}}` and `{{end}}`, which closes any of them."
            ),
            RetiredViewForm(RetiredForm::Element(tag)) => format!(
                "`{tag}: …` is the retired `Render:` way to write an element. In a `View:` an element \
                 is markup — `<{tag}>…</{tag}>` — and a component is placed by its name: \
                 `<Card title=\"Hello\"/>`."
            ),
            RetiredViewForm(RetiredForm::Stylesheet) => {
                "`css \"…\"` is the retired `Render:` way to write a stylesheet. A component's CSS goes \
                 in its own `Style:` section, beside `View:`, scoped to the component."
                    .into()
            }
        }
    }
}

const TAG_WORDS: &[&str] = &[
    "if", "else", "end", "for", "inspect", "when", "let", "extends", "block", "children", "raw",
];

/// Tag words other template languages use, and the LGX tag each one means. Each is refused with
/// the LGX spelling rather than read as a hole naming a variable called `elif`.
pub const TAG_ALIASES: &[(&str, &str)] = &[
    ("elif", "else if"),
    ("elsif", "else if"),
    ("elseif", "else if"),
    ("otherwise", "else"),
    ("endif", "end"),
    ("endfor", "end"),
];

enum Frame {
    Element {
        tag: String,
        attrs: Vec<TAttr>,
        children: Vec<TNode>,
        start: usize,
        verbatim: bool,
    },
    Component {
        name: String,
        props: Vec<TAttr>,
        children: Vec<TNode>,
        start: usize,
    },
    If {
        arms: Vec<(TExpr, Vec<TNode>)>,
        cond: TExpr,
        current: Vec<TNode>,
        otherwise: Option<Vec<TNode>>,
        in_else: bool,
        start: usize,
    },
    For {
        pattern: String,
        iterable: TExpr,
        key: Option<TExpr>,
        index: Option<String>,
        body: Vec<TNode>,
        current: Vec<TNode>,
        in_else: bool,
        start: usize,
    },
    Inspect {
        scrutinee: TExpr,
        arms: Vec<(String, Vec<TNode>)>,
        current: Option<(String, Vec<TNode>)>,
        start: usize,
    },
    Block {
        name: String,
        children: Vec<TNode>,
        start: usize,
    },
}

impl Frame {
    fn tag_word(&self) -> &'static str {
        match self {
            Frame::If { .. } => "if",
            Frame::For { .. } => "for each",
            Frame::Inspect { .. } => "inspect",
            Frame::Block { .. } => "block",
            Frame::Element { .. } | Frame::Component { .. } => "",
        }
    }
    fn start(&self) -> usize {
        match self {
            Frame::Element { start, .. }
            | Frame::Component { start, .. }
            | Frame::If { start, .. }
            | Frame::For { start, .. }
            | Frame::Inspect { start, .. }
            | Frame::Block { start, .. } => *start,
        }
    }
}

struct P {
    stack: Vec<Frame>,
    root: Vec<TNode>,
    saw_content: bool,
}

impl P {
    fn push_node(&mut self, node: TNode, at: usize) -> Result<(), TemplateError> {
        match self.stack.last_mut() {
            None => {
                if !matches!(node, TNode::Extends { .. }) {
                    self.saw_content = true;
                }
                self.root.push(node);
                Ok(())
            }
            Some(Frame::Element { children, .. })
            | Some(Frame::Component { children, .. })
            | Some(Frame::Block { children, .. }) => {
                children.push(node);
                Ok(())
            }
            Some(Frame::If { current, .. }) | Some(Frame::For { current, .. }) => {
                current.push(node);
                Ok(())
            }
            Some(Frame::Inspect { current, .. }) => match current {
                Some((_, body)) => {
                    body.push(node);
                    Ok(())
                }
                None => {
                    // Only whitespace may sit between `{inspect v:}` and its first `{when …:}`.
                    if let TNode::Text { text, .. } = &node {
                        if text.trim().is_empty() {
                            return Ok(());
                        }
                    }
                    Err(TemplateError {
                        kind: TemplateErrorKind::MalformedTag("inspect".into()),
                        span: Span::new(at, at),
                    })
                }
            },
        }
    }

    fn verbatim(&self) -> bool {
        self.stack
            .iter()
            .any(|f| matches!(f, Frame::Element { verbatim: true, .. }))
    }
}

/// Parse a raw view body into template nodes.
pub fn parse(src: &str) -> Result<Vec<TNode>, TemplateError> {
    let tokens = html::tokenize(src).map_err(|e| TemplateError {
        kind: TemplateErrorKind::Html(e.kind),
        span: e.span,
    })?;
    let mut p = P {
        stack: Vec::new(),
        root: Vec::new(),
        saw_content: false,
    };
    for tok in tokens {
        match tok {
            HtmlToken::Text { text, span } => {
                if p.verbatim() {
                    p.push_node(TNode::Text { text, span }, span.start)?;
                    continue;
                }
                if let Some((form, at)) = retired_line_in(src, span) {
                    return Err(TemplateError {
                        kind: TemplateErrorKind::RetiredViewForm(form),
                        span: at,
                    });
                }
                if let Some(t) = collapse(&text) {
                    p.push_node(TNode::Text { text: t, span }, span.start)?;
                }
            }
            HtmlToken::StartTag {
                name,
                attrs,
                self_closing,
                span,
            } => {
                let converted = convert_attrs(&attrs)?;
                if html::is_component_name(&name) {
                    if self_closing {
                        p.push_node(
                            TNode::Component {
                                name,
                                props: converted,
                                children: Vec::new(),
                                span,
                            },
                            span.start,
                        )?;
                    } else {
                        p.stack.push(Frame::Component {
                            name,
                            props: converted,
                            children: Vec::new(),
                            start: span.start,
                        });
                    }
                } else if self_closing {
                    p.push_node(
                        TNode::Element {
                            tag: name,
                            attrs: converted,
                            children: Vec::new(),
                            span,
                        },
                        span.start,
                    )?;
                } else {
                    let verbatim =
                        name.eq_ignore_ascii_case("pre") || name.eq_ignore_ascii_case("textarea");
                    p.stack.push(Frame::Element {
                        tag: name,
                        attrs: converted,
                        children: Vec::new(),
                        start: span.start,
                        verbatim,
                    });
                }
            }
            HtmlToken::EndTag { name, span } => match p.stack.pop() {
                None => {
                    if html::is_void(&name) {
                        return Err(TemplateError {
                            kind: TemplateErrorKind::UnexpectedEndTagOfVoid(name),
                            span,
                        });
                    }
                    return Err(TemplateError {
                        kind: TemplateErrorKind::StrayEndTag(name),
                        span,
                    });
                }
                Some(Frame::Element {
                    tag,
                    attrs,
                    children,
                    start,
                    ..
                }) => {
                    if !tag.eq_ignore_ascii_case(&name) {
                        if html::is_void(&name) {
                            return Err(TemplateError {
                                kind: TemplateErrorKind::UnexpectedEndTagOfVoid(name),
                                span,
                            });
                        }
                        return Err(TemplateError {
                            kind: TemplateErrorKind::MismatchedEndTag {
                                expected: tag,
                                found: name,
                            },
                            span,
                        });
                    }
                    p.push_node(
                        TNode::Element {
                            tag,
                            attrs,
                            children,
                            span: Span::new(start, span.end),
                        },
                        start,
                    )?;
                }
                Some(Frame::Component {
                    name: cname,
                    props,
                    children,
                    start,
                }) => {
                    if cname != name {
                        return Err(TemplateError {
                            kind: TemplateErrorKind::MismatchedEndTag {
                                expected: cname,
                                found: name,
                            },
                            span,
                        });
                    }
                    p.push_node(
                        TNode::Component {
                            name: cname,
                            props,
                            children,
                            span: Span::new(start, span.end),
                        },
                        start,
                    )?;
                }
                Some(other) => {
                    return Err(TemplateError {
                        kind: TemplateErrorKind::UnclosedTag(other.tag_word().to_string()),
                        span: Span::new(other.start(), span.end),
                    });
                }
            },
            HtmlToken::Brace { inner, span } => handle_brace(&mut p, &inner, span)?,
        }
    }
    if let Some(top) = p.stack.pop() {
        let start = top.start();
        return Err(match top {
            Frame::Element { tag, .. } => TemplateError {
                kind: TemplateErrorKind::UnclosedElement(tag),
                span: Span::new(start, src.len()),
            },
            Frame::Component { name, .. } => TemplateError {
                kind: TemplateErrorKind::UnclosedElement(name),
                span: Span::new(start, src.len()),
            },
            other => TemplateError {
                kind: TemplateErrorKind::UnclosedTag(other.tag_word().to_string()),
                span: Span::new(start, src.len()),
            },
        });
    }
    Ok(p.root)
}

/// Whitespace rule for non-verbatim text: whitespace-only text containing a newline is
/// formatting (dropped); otherwise runs of whitespace collapse to one space.
fn collapse(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return if text.contains('\n') {
            None
        } else {
            Some(" ".to_string())
        };
    }
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    Some(out)
}

fn convert_attrs(attrs: &[HtmlAttr]) -> Result<Vec<TAttr>, TemplateError> {
    let mut out = Vec::with_capacity(attrs.len());
    for a in attrs {
        let single_brace = |a: &HtmlAttr| -> Option<TExpr> {
            match &a.value {
                Some(AttrValue::Parts(parts)) if parts.len() == 1 => match &parts[0] {
                    AttrPart::Brace { inner, span } => Some(texpr(inner, span.start + 1)),
                    _ => None,
                },
                _ => None,
            }
        };
        if let Some(rest) = a.name.strip_prefix("on-") {
            let mut segs = rest.split('.');
            let event = segs.next().unwrap_or("").to_string();
            let words: Vec<&str> = segs.collect();
            let expr = single_brace(a).ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedDirective(a.name.clone()),
                span: a.span,
            })?;
            if event.is_empty() || expr.text.trim().is_empty() {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedDirective(a.name.clone()),
                    span: a.span,
                });
            }
            let modifiers = html::classify_modifiers(&event, &words).map_err(|(at, e)| {
                // The caret covers the refused `.qualifier` inside the attribute name.
                let start = a.span.start
                    + "on-".len()
                    + event.len()
                    + words[..at].iter().map(|w| w.len() + 1).sum::<usize>();
                TemplateError {
                    kind: TemplateErrorKind::EventModifier(e),
                    span: Span::new(start, start + 1 + words[at].len()),
                }
            })?;
            out.push(TAttr::On {
                event,
                modifiers,
                expr,
                span: a.span,
            });
        } else if a.name == "bind" {
            let expr = single_brace(a).ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedDirective("bind".into()),
                span: a.span,
            })?;
            let field = expr.text.trim().to_string();
            if field.is_empty() || !field.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedDirective("bind".into()),
                    span: a.span,
                });
            }
            out.push(TAttr::Bind {
                field,
                at: expr.offset,
                span: a.span,
            });
        } else if let Some(class) = a.name.strip_prefix("class:") {
            let expr = single_brace(a).ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedDirective(a.name.clone()),
                span: a.span,
            })?;
            if class.is_empty() {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedDirective(a.name.clone()),
                    span: a.span,
                });
            }
            out.push(TAttr::Class {
                name: class.to_string(),
                expr,
                span: a.span,
            });
        } else {
            match &a.value {
                None => out.push(TAttr::Static {
                    name: a.name.clone(),
                    value: None,
                    span: a.span,
                }),
                Some(AttrValue::Static(v)) => out.push(TAttr::Static {
                    name: a.name.clone(),
                    value: Some(v.clone()),
                    span: a.span,
                }),
                Some(AttrValue::Parts(parts)) => out.push(TAttr::Dynamic {
                    name: a.name.clone(),
                    parts: parts.clone(),
                    span: a.span,
                }),
            }
        }
    }
    Ok(out)
}

/// A `TExpr` for `text` whose first byte sits at `base` in the source (leading whitespace
/// is trimmed and the offset advanced past it).
fn texpr(text: &str, base: usize) -> TExpr {
    let trimmed_start = text.len() - text.trim_start().len();
    TExpr {
        text: text.trim().to_string(),
        offset: base + trimmed_start,
    }
}

fn handle_brace(p: &mut P, inner: &str, span: Span) -> Result<(), TemplateError> {
    let body = inner.trim();
    let base = span.start + 1; // first byte after `{`
    if body.is_empty() {
        return Err(TemplateError {
            kind: TemplateErrorKind::EmptyHole,
            span,
        });
    }
    if body.starts_with('#') && body.ends_with('#') && body.len() >= 2 {
        return Ok(());
    }
    let first = body
        .split(|c: char| c.is_whitespace() || c == ':')
        .next()
        .unwrap_or("");
    if let Some((_, canonical)) = TAG_ALIASES.iter().find(|(alias, _)| *alias == first) {
        let rest = body[first.len()..].trim_start();
        let spelled = if rest.is_empty() || rest.starts_with(':') {
            format!("{{{canonical}{rest}}}")
        } else {
            format!("{{{canonical} {rest}}}")
        };
        return Err(TemplateError {
            kind: TemplateErrorKind::NonCanonicalTag {
                written: format!("{{{body}}}"),
                canonical: spelled,
            },
            span,
        });
    }
    if !TAG_WORDS.contains(&first) {
        if body.ends_with(':') {
            return Err(TemplateError {
                kind: TemplateErrorKind::TagLikeHole(body.to_string()),
                span,
            });
        }
        return hole(p, inner, base, span, false);
    }
    let rest_after_first = body[first.len()..].trim_start();
    match first {
        "end" => {
            if !rest_after_first.is_empty() {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("end".into()),
                    span,
                });
            }
            close_tag(p, span)
        }
        "if" => {
            let cond = strip_colon(rest_after_first).ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedTag("if".into()),
                span,
            })?;
            if cond.trim().is_empty() {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("if".into()),
                    span,
                });
            }
            let cond = texpr(cond, base + offset_of(inner, cond));
            p.stack.push(Frame::If {
                arms: Vec::new(),
                cond,
                current: Vec::new(),
                otherwise: None,
                in_else: false,
                start: span.start,
            });
            Ok(())
        }
        "else" => {
            if let Some(rest) = rest_after_first.strip_prefix("if") {
                let cond = strip_colon(rest.trim_start()).ok_or(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("else if".into()),
                    span,
                })?;
                if cond.trim().is_empty() {
                    return Err(TemplateError {
                        kind: TemplateErrorKind::MalformedTag("else if".into()),
                        span,
                    });
                }
                let cond = texpr(cond, base + offset_of(inner, cond));
                match p.stack.last_mut() {
                    Some(Frame::If {
                        arms,
                        cond: cur_cond,
                        current,
                        in_else: false,
                        ..
                    }) => {
                        let finished = std::mem::take(current);
                        let prev = std::mem::replace(cur_cond, cond);
                        arms.push((prev, finished));
                        Ok(())
                    }
                    Some(Frame::If { in_else: true, .. }) => Err(TemplateError {
                        kind: TemplateErrorKind::DuplicateElse,
                        span,
                    }),
                    _ => Err(TemplateError {
                        kind: TemplateErrorKind::StrayElse,
                        span,
                    }),
                }
            } else {
                if strip_colon(rest_after_first).map(|s| s.trim().is_empty()) != Some(true) {
                    return Err(TemplateError {
                        kind: TemplateErrorKind::MalformedTag("else".into()),
                        span,
                    });
                }
                match p.stack.last_mut() {
                    Some(Frame::If {
                        arms,
                        cond,
                        current,
                        otherwise,
                        in_else,
                        ..
                    }) => {
                        if *in_else {
                            return Err(TemplateError {
                                kind: TemplateErrorKind::DuplicateElse,
                                span,
                            });
                        }
                        let finished = std::mem::take(current);
                        arms.push((cond.clone(), finished));
                        *otherwise = Some(Vec::new());
                        *in_else = true;
                        Ok(())
                    }
                    Some(Frame::For {
                        body,
                        current,
                        in_else,
                        ..
                    }) => {
                        if *in_else {
                            return Err(TemplateError {
                                kind: TemplateErrorKind::DuplicateElse,
                                span,
                            });
                        }
                        *body = std::mem::take(current);
                        *in_else = true;
                        Ok(())
                    }
                    _ => Err(TemplateError {
                        kind: TemplateErrorKind::StrayElse,
                        span,
                    }),
                }
            }
        }
        "for" => {
            let rest = rest_after_first
                .strip_prefix("each")
                .map(str::trim_start)
                .ok_or(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("for each".into()),
                    span,
                })?;
            let rest = strip_colon(rest).ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedTag("for each".into()),
                span,
            })?;
            let in_pos = find_top_level_word(rest, " in ").ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedTag("for each".into()),
                span,
            })?;
            let pattern = rest[..in_pos].trim().to_string();
            let after_in = &rest[in_pos + 4..];
            let clauses = split_top_level(after_in, ',');
            let iterable_text = clauses.first().copied().unwrap_or("").trim();
            if pattern.is_empty() || iterable_text.is_empty() {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("for each".into()),
                    span,
                });
            }
            let iterable = texpr(iterable_text, base + offset_of(inner, iterable_text));
            let mut key = None;
            let mut index = None;
            for clause in clauses.iter().skip(1) {
                let c = clause.trim();
                if let Some(k) = c.strip_prefix("keyed by") {
                    let k = k.trim();
                    if k.is_empty() {
                        return Err(TemplateError {
                            kind: TemplateErrorKind::MalformedTag("for each".into()),
                            span,
                        });
                    }
                    key = Some(texpr(k, base + offset_of(inner, k)));
                } else if let Some(i) = c.strip_prefix("at index") {
                    let i = i.trim();
                    if i.is_empty() {
                        return Err(TemplateError {
                            kind: TemplateErrorKind::MalformedTag("for each".into()),
                            span,
                        });
                    }
                    index = Some(i.to_string());
                } else {
                    return Err(TemplateError {
                        kind: TemplateErrorKind::MalformedTag("for each".into()),
                        span,
                    });
                }
            }
            p.stack.push(Frame::For {
                pattern,
                iterable,
                key,
                index,
                body: Vec::new(),
                current: Vec::new(),
                in_else: false,
                start: span.start,
            });
            Ok(())
        }
        "inspect" => {
            let s = strip_colon(rest_after_first).ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedTag("inspect".into()),
                span,
            })?;
            if s.trim().is_empty() {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("inspect".into()),
                    span,
                });
            }
            let scrutinee = texpr(s, base + offset_of(inner, s));
            p.stack.push(Frame::Inspect {
                scrutinee,
                arms: Vec::new(),
                current: None,
                start: span.start,
            });
            Ok(())
        }
        "when" => {
            let pat = strip_colon(rest_after_first).ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedTag("when".into()),
                span,
            })?;
            if pat.trim().is_empty() {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("when".into()),
                    span,
                });
            }
            match p.stack.last_mut() {
                Some(Frame::Inspect { arms, current, .. }) => {
                    if let Some(done) = current.take() {
                        arms.push(done);
                    }
                    *current = Some((pat.trim().to_string(), Vec::new()));
                    Ok(())
                }
                _ => Err(TemplateError {
                    kind: TemplateErrorKind::StrayWhen,
                    span,
                }),
            }
        }
        "let" => {
            let be = find_top_level_word(rest_after_first, " be ").ok_or(TemplateError {
                kind: TemplateErrorKind::MalformedTag("let".into()),
                span,
            })?;
            let name = rest_after_first[..be].trim().to_string();
            let e = rest_after_first[be + 4..].trim();
            if name.is_empty() || e.is_empty() || e.ends_with(':') {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("let".into()),
                    span,
                });
            }
            let expr = texpr(e, base + offset_of(inner, e));
            p.push_node(TNode::Let { name, expr, span }, span.start)
        }
        "extends" => {
            let name = rest_after_first.trim();
            if name.is_empty() || name.contains(char::is_whitespace) || name.ends_with(':') {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("extends".into()),
                    span,
                });
            }
            if !p.stack.is_empty() || p.saw_content {
                return Err(TemplateError {
                    kind: TemplateErrorKind::ExtendsNotFirst,
                    span,
                });
            }
            p.push_node(
                TNode::Extends {
                    name: name.to_string(),
                    span,
                },
                span.start,
            )
        }
        "block" => {
            let has_colon = rest_after_first.ends_with(':');
            let name = rest_after_first.trim_end_matches(':').trim();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("block".into()),
                    span,
                });
            }
            if has_colon {
                p.stack.push(Frame::Block {
                    name: name.to_string(),
                    children: Vec::new(),
                    start: span.start,
                });
                Ok(())
            } else {
                p.push_node(
                    TNode::Block {
                        name: name.to_string(),
                        default: None,
                        span,
                    },
                    span.start,
                )
            }
        }
        "children" => {
            if !rest_after_first.is_empty() {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("children".into()),
                    span,
                });
            }
            p.push_node(TNode::Children { span }, span.start)
        }
        "raw" => {
            if rest_after_first.is_empty() || rest_after_first.ends_with(':') {
                return Err(TemplateError {
                    kind: TemplateErrorKind::MalformedTag("raw".into()),
                    span,
                });
            }
            hole(
                p,
                rest_after_first,
                base + offset_of(inner, rest_after_first),
                span,
                true,
            )
        }
        _ => unreachable!("every TAG_WORD is handled above"),
    }
}

fn close_tag(p: &mut P, span: Span) -> Result<(), TemplateError> {
    // `{end}` with no template tag open anywhere closes nothing — even inside an element.
    let any_tag_open = p
        .stack
        .iter()
        .any(|f| !matches!(f, Frame::Element { .. } | Frame::Component { .. }));
    if !any_tag_open {
        return Err(TemplateError {
            kind: TemplateErrorKind::UnexpectedEnd,
            span,
        });
    }
    match p.stack.pop() {
        None => Err(TemplateError {
            kind: TemplateErrorKind::UnexpectedEnd,
            span,
        }),
        Some(Frame::If {
            mut arms,
            cond,
            current,
            otherwise,
            in_else,
            start,
        }) => {
            // In the else arm, `current` IS the else body (the last conditional arm was pushed at
            // the `{else:}`); otherwise `current` is the last conditional arm.
            let otherwise = if in_else {
                Some(current)
            } else {
                arms.push((cond, current));
                otherwise
            };
            let node = TNode::If {
                arms,
                otherwise,
                span: Span::new(start, span.end),
            };
            p.push_node(node, start)
        }
        Some(Frame::For {
            pattern,
            iterable,
            key,
            index,
            body,
            current,
            in_else,
            start,
        }) => {
            let (body, empty) = if in_else {
                (body, Some(current))
            } else {
                (current, None)
            };
            p.push_node(
                TNode::For {
                    pattern,
                    iterable,
                    key,
                    index,
                    body,
                    empty,
                    span: Span::new(start, span.end),
                },
                start,
            )
        }
        Some(Frame::Inspect {
            scrutinee,
            mut arms,
            current,
            start,
        }) => {
            if let Some(done) = current {
                arms.push(done);
            }
            p.push_node(
                TNode::Inspect {
                    scrutinee,
                    arms,
                    span: Span::new(start, span.end),
                },
                start,
            )
        }
        Some(Frame::Block {
            name,
            children,
            start,
        }) => p.push_node(
            TNode::Block {
                name,
                default: Some(children),
                span: Span::new(start, span.end),
            },
            start,
        ),
        Some(Frame::Element { tag, start, .. }) => Err(TemplateError {
            kind: TemplateErrorKind::UnclosedElement(tag),
            span: Span::new(start, span.end),
        }),
        Some(Frame::Component { name, start, .. }) => Err(TemplateError {
            kind: TemplateErrorKind::UnclosedElement(name),
            span: Span::new(start, span.end),
        }),
    }
}

fn hole(p: &mut P, text: &str, base: usize, span: Span, raw: bool) -> Result<(), TemplateError> {
    let segments = split_top_level(text, '|');
    let mut iter = segments.iter();
    let head = iter.next().copied().unwrap_or("");
    if head.trim().is_empty() {
        return Err(TemplateError {
            kind: TemplateErrorKind::EmptyHole,
            span,
        });
    }
    let expr = texpr(head, base + offset_of(text, head));
    let mut filters = Vec::new();
    for seg in iter {
        let seg_trim = seg.trim();
        if seg_trim.is_empty() {
            return Err(TemplateError {
                kind: TemplateErrorKind::EmptyHole,
                span,
            });
        }
        let name_end = seg_trim.find(char::is_whitespace).unwrap_or(seg_trim.len());
        let name = seg_trim[..name_end].to_string();
        let args_text = &seg_trim[name_end..];
        let args: Vec<TExpr> = split_top_level_ws(args_text)
            .into_iter()
            .map(|a| texpr(a, base + offset_of(text, a)))
            .collect();
        filters.push(TFilter { name, args });
    }
    p.push_node(
        TNode::Hole {
            expr,
            filters,
            raw,
            span,
        },
        span.start,
    )
}

/// Byte offset of the slice `part` inside `whole` (it is always a subslice).
fn offset_of(whole: &str, part: &str) -> usize {
    let w = whole.as_ptr() as usize;
    let p = part.as_ptr() as usize;
    if p >= w && p <= w + whole.len() {
        p - w
    } else {
        0
    }
}

fn strip_colon(s: &str) -> Option<&str> {
    s.trim_end().strip_suffix(':')
}

/// Position of `needle` (a space-delimited word like `" in "`) at the top level of `s`.
fn find_top_level_word(s: &str, needle: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            // ★ COMPARED AS BYTES, NEVER SLICED. `i` walks byte by byte, so it lands inside a
            // multi-byte character — and `s[i..]` there is a PANIC ("byte index N is not a char
            // boundary"), which took down `largo check`, `largo build`, the LSP and the dev
            // server on `{for each café in cafés:}`. A byte-slice comparison cannot panic, and
            // it cannot match falsely either: every needle here begins with an ASCII space, and
            // no continuation byte is ASCII. See `lgx_non_ascii_lock`.
            _ if depth == 0 && b[i..].starts_with(needle.as_bytes()) => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split `s` on `delim` outside strings and brackets, as subslices of `s` (so byte offsets
/// into the template survive).
fn split_top_level(s: &str, delim: char) -> Vec<&str> {
    let b = s.as_bytes();
    let d = delim as u8;
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            c if c == d && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&s[start.min(s.len())..]);
    out
}

/// Split filter arguments on whitespace outside strings and brackets, as subslices of `s`.
fn split_top_level_ws(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => {
                if start.is_none() {
                    start = Some(i);
                }
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b')' | b']' | b'}' => depth -= 1,
            _ if c.is_ascii_whitespace() && depth == 0 => {
                if let Some(st) = start.take() {
                    out.push(&s[st..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
        i += 1;
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Printing — the inverse of `parse`, for round-trip locks, `largo expand`, and formatting.
// ---------------------------------------------------------------------------------------------

/// Print a template tree back to LGX source. `parse(print(t))` yields `t` again (spans
/// aside), and printing is idempotent; text and attribute values are escaped so that no
/// character can be re-read as markup or a hole.
pub fn print(nodes: &[TNode]) -> String {
    let mut out = String::new();
    print_nodes(nodes, &mut out);
    out
}

fn print_nodes(nodes: &[TNode], out: &mut String) {
    for n in nodes {
        print_node(n, out);
    }
}

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '{' => out.push_str("&#123;"),
            '}' => out.push_str("&#125;"),
            _ => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '{' => out.push_str("&#123;"),
            '}' => out.push_str("&#125;"),
            _ => out.push(c),
        }
    }
}

fn print_attrs(attrs: &[TAttr], out: &mut String) {
    for a in attrs {
        out.push(' ');
        match a {
            TAttr::Static {
                name, value: None, ..
            } => out.push_str(name),
            TAttr::Static {
                name,
                value: Some(v),
                ..
            } => {
                out.push_str(name);
                out.push_str("=\"");
                escape_attr(v, out);
                out.push('"');
            }
            TAttr::Dynamic { name, parts, .. } => {
                out.push_str(name);
                out.push('=');
                if let [AttrPart::Brace { inner, .. }] = parts.as_slice() {
                    out.push('{');
                    out.push_str(inner.trim());
                    out.push('}');
                } else {
                    out.push('"');
                    for p in parts {
                        match p {
                            AttrPart::Text(t) => escape_attr(t, out),
                            AttrPart::Brace { inner, .. } => {
                                out.push('{');
                                out.push_str(inner.trim());
                                out.push('}');
                            }
                        }
                    }
                    out.push('"');
                }
            }
            TAttr::On {
                event,
                modifiers,
                expr,
                ..
            } => {
                out.push_str("on-");
                out.push_str(event);
                for m in modifiers {
                    out.push('.');
                    out.push_str(&m.word());
                }
                out.push_str("={");
                out.push_str(&expr.text);
                out.push('}');
            }
            TAttr::Bind { field, .. } => {
                out.push_str("bind={");
                out.push_str(field);
                out.push('}');
            }
            TAttr::Class { name, expr, .. } => {
                out.push_str("class:");
                out.push_str(name);
                out.push_str("={");
                out.push_str(&expr.text);
                out.push('}');
            }
        }
    }
}

fn print_node(n: &TNode, out: &mut String) {
    match n {
        TNode::Element {
            tag,
            attrs,
            children,
            ..
        } => {
            out.push('<');
            out.push_str(tag);
            print_attrs(attrs, out);
            out.push('>');
            if html::is_void(tag) {
                return;
            }
            print_nodes(children, out);
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        TNode::Text { text, .. } => escape_text(text, out),
        TNode::Hole {
            expr, filters, raw, ..
        } => {
            out.push('{');
            if *raw {
                out.push_str("raw ");
            }
            out.push_str(&expr.text);
            for f in filters {
                out.push_str(" | ");
                out.push_str(&f.name);
                for a in &f.args {
                    out.push(' ');
                    out.push_str(&a.text);
                }
            }
            out.push('}');
        }
        TNode::If {
            arms, otherwise, ..
        } => {
            for (i, (cond, body)) in arms.iter().enumerate() {
                out.push_str(if i == 0 { "{if " } else { "{else if " });
                out.push_str(&cond.text);
                out.push_str(":}");
                print_nodes(body, out);
            }
            if let Some(o) = otherwise {
                out.push_str("{else:}");
                print_nodes(o, out);
            }
            out.push_str("{end}");
        }
        TNode::For {
            pattern,
            iterable,
            key,
            index,
            body,
            empty,
            ..
        } => {
            out.push_str("{for each ");
            out.push_str(pattern);
            out.push_str(" in ");
            out.push_str(&iterable.text);
            if let Some(k) = key {
                out.push_str(", keyed by ");
                out.push_str(&k.text);
            }
            if let Some(i) = index {
                out.push_str(", at index ");
                out.push_str(i);
            }
            out.push_str(":}");
            print_nodes(body, out);
            if let Some(e) = empty {
                out.push_str("{else:}");
                print_nodes(e, out);
            }
            out.push_str("{end}");
        }
        TNode::Inspect {
            scrutinee, arms, ..
        } => {
            out.push_str("{inspect ");
            out.push_str(&scrutinee.text);
            out.push_str(":}");
            for (pat, body) in arms {
                out.push_str("{when ");
                out.push_str(pat);
                out.push_str(":}");
                print_nodes(body, out);
            }
            out.push_str("{end}");
        }
        TNode::Let { name, expr, .. } => {
            out.push_str("{let ");
            out.push_str(name);
            out.push_str(" be ");
            out.push_str(&expr.text);
            out.push('}');
        }
        TNode::Extends { name, .. } => {
            out.push_str("{extends ");
            out.push_str(name);
            out.push('}');
        }
        TNode::Block { name, default, .. } => {
            out.push_str("{block ");
            out.push_str(name);
            match default {
                None => out.push('}'),
                Some(d) => {
                    out.push_str(":}");
                    print_nodes(d, out);
                    out.push_str("{end}");
                }
            }
        }
        TNode::Children { .. } => out.push_str("{children}"),
        TNode::Component {
            name,
            props,
            children,
            ..
        } => {
            out.push('<');
            out.push_str(name);
            print_attrs(props, out);
            if children.is_empty() {
                out.push_str("/>");
            } else {
                out.push('>');
                print_nodes(children, out);
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
        }
    }
}
