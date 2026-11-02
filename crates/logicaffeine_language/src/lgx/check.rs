//! Structural and accessibility checks over a parsed template tree — the questions a
//! template can answer before any LOGOS expression inside it is parsed.
//!
//! Errors are shapes that cannot mean anything: content beside `{block}`s in a view that
//! `{extends}` a layout, a block name defined twice, `{children}` twice, `bind` on
//! something that is not a form control, `class:` on a component (a component takes
//! props, not classes). Warnings are the accessibility mistakes that ship far too often
//! — an image with no `alt`, a button that announces nothing, a form control with no
//! label, a link with nowhere to go. Every finding carries the span and a message that
//! names the fix.

use super::template::{TAttr, TNode};
use crate::token::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingKind {
    /// Loose content at the top level of a view that `{extends}` a layout.
    ContentOutsideBlock,
    /// The same `{block name}` twice in one template.
    DuplicateBlock(String),
    /// `{children}` more than once.
    DuplicateChildren,
    /// `bind={…}` on an element that is not `<input>`, `<textarea>` or `<select>`.
    BindOnNonInput(String),
    /// `class:name={…}` on a component.
    ClassToggleOnComponent(String),
    /// `<img>` without an `alt` attribute (an empty `alt=""` marks it decorative and is fine).
    ImageWithoutAlt,
    /// `<button>` with no text, no hole, and no `aria-label`/`aria-labelledby`/`title`.
    ButtonWithoutName,
    /// A form control with no `<label for>`, no wrapping `<label>`, and no `aria-label`.
    InputWithoutLabel,
    /// `<a>` without `href`.
    LinkWithoutHref,
    /// A `<form>` whose method changes state, with no `_csrf` field in it.
    FormWithoutCsrf(String),
    /// An inline event-handler attribute (`onclick`, `onerror`, …) — a JavaScript sink.
    InlineEventHandler(String),
    /// A `bind` on a form control the browser will not let a page write — `<input type="file">`
    /// and `<select multiple>`.
    ///
    /// ★ CHECKED HERE, NOT AT A TIER, because the mistake is semantic rather than a limit of any
    /// one backend: a file input's value cannot be set by a page ANYWHERE, and a multiple select's
    /// value is a list on every tier. A tier-specific gate would let any tier outside it accept it
    /// happily — the binding written as a plain text `setValue` does nothing and reports nothing.
    ///
    /// The wording comes from `BoundInput::refusal()` so the check and the lowering cannot drift.
    BindNotHonoured(&'static str),
    /// Markup a browser's HTML parser does not keep as written — `what` (an element, or `text`)
    /// where the parser moves, wraps or closes it — so the document a browser builds from the
    /// pre-rendered page is not the page the view draws, and the page is drawn again as it loads.
    RewrittenByParser(ParserRewrite, String),
}

/// What a browser's HTML parser does to markup it does not keep as written — see
/// [`FindingKind::RewrittenByParser`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserRewrite {
    /// A `<tr>` directly in a `<table>` gets a `<tbody>` around it.
    WrapsRowInTbody,
    /// A `<td>`/`<th>` directly in a table or a table section gets a `<tr>` around it.
    WrapsCellInRow,
    /// A `<col>` directly in a `<table>` gets a `<colgroup>` around it.
    WrapsColInColgroup,
    /// Content a table or table row cannot hold is moved out, in front of the table.
    MovesOutOfTable,
    /// A block element closes the `<p>` it is written in.
    ClosesParagraph,
    /// A link inside a link closes the outer one.
    ClosesLink,
    /// A form inside a form is dropped.
    DropsNestedForm,
    /// A button inside a button closes the outer one.
    ClosesButton,
    /// A list item, or a `<dt>`/`<dd>`, closes the one it is written in.
    ClosesItem,
    /// An `<option>` or `<optgroup>` closes the option it is written in.
    ClosesOption,
    /// A heading closes the heading it is written in.
    ClosesHeading,
}

impl ParserRewrite {
    /// What the parser does, as the finding says it, and the fix.
    fn says(self) -> (&'static str, &'static str) {
        use ParserRewrite::*;
        match self {
            WrapsRowInTbody => ("wraps the row in a `<tbody>`", "Put the rows in `<tbody>…</tbody>` (or `<thead>`/`<tfoot>`)."),
            WrapsCellInRow => ("wraps the cell in a `<tr>`", "Put the cells in a `<tr>`."),
            WrapsColInColgroup => ("wraps it in a `<colgroup>`", "Put the `<col>`s in a `<colgroup>`."),
            MovesOutOfTable => ("moves it out, in front of the table", "Put it inside a cell (`<td>`), or outside the table."),
            ClosesParagraph => ("closes the `<p>` before it, so it and what follows it land outside the paragraph", "Use a `<div>` in place of the `<p>`, or keep the paragraph to text and inline elements."),
            ClosesLink => ("closes the outer link first, so the two links end up side by side", "Links cannot nest: move the inner one out, or make the outer one a `<div>`."),
            DropsNestedForm => ("drops the inner form's tag", "Forms cannot nest: close the outer form first."),
            ClosesButton => ("closes the outer button first", "Buttons cannot nest: move the inner one out."),
            ClosesItem => ("closes the outer item first", "Nest a list (`<ul>`, `<ol>`, `<dl>`) between the two items."),
            ClosesOption => ("closes the outer option first", "Options cannot nest: group them with `<optgroup>` inside the `<select>`."),
            ClosesHeading => ("closes the outer heading first", "Headings cannot nest: use a `<span>` inside the heading."),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: FindingKind,
    pub severity: Severity,
    pub span: Span,
    pub message: String,
}

fn finding(kind: FindingKind, span: Span) -> Finding {
    use FindingKind::*;
    let (severity, message) = match &kind {
        ContentOutsideBlock => (Severity::Error, "A view that `{extends}` a layout fills the layout's blocks: put this content inside a `{block name:} … {end}` (or drop the `{extends}`).".to_string()),
        DuplicateBlock(n) => (Severity::Error, format!("`{{block {n}}}` is defined twice in this template; a block name is a slot, so name each one once.")),
        DuplicateChildren => (Severity::Error, "`{children}` can appear once: it is where the caller's content goes. For several slots, use named `{block}`s.".to_string()),
        BindOnNonInput(tag) => (Severity::Error, format!("`bind={{…}}` two-way binds a form control's value; `<{tag}>` has none. Use it on `<input>`, `<textarea>` or `<select>`, or show the value with a hole `{{…}}`.")),
        ClassToggleOnComponent(name) => (Severity::Error, format!("`class:` toggles a class on an HTML element; `<{name}>` is a component and takes props. Pass the flag as a prop (`<{name} done={{…}}>`) and toggle inside its view.")),
        ImageWithoutAlt => (Severity::Warning, "This `<img>` has no `alt`. Describe it (`alt=\"A cat\"`), or mark it decorative with `alt=\"\"` so screen readers skip it.".to_string()),
        ButtonWithoutName => (Severity::Warning, "This `<button>` announces nothing: give it text, a `{…}` hole, or `aria-label=\"…\"`.".to_string()),
        InputWithoutLabel => (Severity::Warning, "This form control has no label. Add `<label for=\"id\">` with a matching `id`, wrap it in `<label>…</label>`, or set `aria-label=\"…\"`.".to_string()),
        LinkWithoutHref => (Severity::Warning, "This `<a>` has no `href`, so it is not a link: add `href=\"…\"`, or use a `<button>` for an action.".to_string()),
        // An ERROR, not a warning, and deliberately so: CSRF protection you have to REMEMBER is
        // not protection. A warning is skipped on the one form somebody added in a hurry, and
        // nothing says so until a visitor's browser is made to submit it from another site.
        FormWithoutCsrf(method) => (Severity::Error, format!("This `<form method=\"{method}\">` changes something, so a browser can be made to submit it from another site. Add the token this page was given: `<input type=\"hidden\" name=\"_csrf\" value={{token}}>`, and check it with `csrfValid` in the route that answers. (A `get` form changes nothing and needs none.)")),
        // An ERROR for the same reason `<script>` is: this is a JavaScript position, and the
        // escaping that protects every other attribute is meaningless in one. A hole here is a
        // stranger's text being EXECUTED, not markup being neutralised.
        //
        // It is also the likelier mistake: the framework's own spelling is `on-click` with a
        // hyphen, and `onclick` is one keystroke away. Written that way it is not wired to
        // anything — it emits a raw handler carrying the stringified value — so the page looks
        // connected and answers nothing, which is why the message leads with the fix.
        BindNotHonoured(why) => (Severity::Error, format!("{why}.")),
        RewrittenByParser(rewrite, what) => {
            let (does, fix) = rewrite.says();
            (Severity::Warning, format!("A browser's HTML parser does not keep {what} where this view puts it: it {does}. The page a browser builds from the pre-rendered markup is then not the page this view draws, so the page is drawn again as it loads, and what a visitor focused or typed first is lost. {fix}"))
        }
        InlineEventHandler(name) => (Severity::Error, format!("`{name}` is an inline JavaScript handler, and a view cannot carry one: behavior belongs in `Actions:`. Write the action and attach it with `on-{}={{…}}` — note the hyphen, which is what tells the framework to wire it up. (Escaping cannot help inside `{name}`: its value is script, not markup.)", name.strip_prefix("on").unwrap_or(name))),
    };
    Finding {
        kind,
        severity,
        span,
        message,
    }
}

/// Does this subtree carry an `<input name="_csrf">`?
///
/// The check is about the FIELD being present, never about what fills it: the token is computed,
/// so its value is a hole in every real page.
fn has_csrf_field(nodes: &[TNode]) -> bool {
    nodes.iter().any(|n| match n {
        TNode::Element {
            tag,
            attrs,
            children,
            ..
        } => {
            (tag.eq_ignore_ascii_case("input")
                && static_attr(attrs, "name").is_some_and(|v| v == "_csrf"))
                || has_csrf_field(children)
        }
        TNode::If {
            arms, otherwise, ..
        } => {
            arms.iter().any(|(_, body)| has_csrf_field(body))
                || otherwise.as_ref().is_some_and(|o| has_csrf_field(o))
        }
        TNode::For { body, empty, .. } => {
            has_csrf_field(body) || empty.as_ref().is_some_and(|e| has_csrf_field(e))
        }
        TNode::Inspect { arms, .. } => arms.iter().any(|(_, body)| has_csrf_field(body)),
        TNode::Block {
            default: Some(body),
            ..
        } => has_csrf_field(body),
        TNode::Component { children, .. } => has_csrf_field(children),
        _ => false,
    })
}

/// Run every check over a template tree.
pub fn check(nodes: &[TNode]) -> Vec<Finding> {
    let mut out = Vec::new();
    // Structural: an extending view carries only blocks at its top level.
    if matches!(nodes.first(), Some(TNode::Extends { .. })) {
        for n in &nodes[1..] {
            match n {
                TNode::Block { .. } => {}
                TNode::Text { text, .. } if text.trim().is_empty() => {}
                other => out.push(finding(FindingKind::ContentOutsideBlock, node_span(other))),
            }
        }
    }
    let mut blocks: Vec<String> = Vec::new();
    let mut children_seen = 0usize;
    let labels_for = collect_label_targets(nodes);
    walk(
        nodes,
        false,
        &labels_for,
        &mut blocks,
        &mut children_seen,
        &mut out,
    );
    parser_rewrites(nodes, &mut Vec::new(), &mut out);
    out
}

/// A table or table section, whose content model the parser enforces as it goes.
const TABLE_PARTS: &[&str] = &["table", "tbody", "thead", "tfoot", "tr"];

/// The elements that close an open `<p>` (the `p` end tag may be omitted before each).
const CLOSES_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "dialog",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "search",
    "section",
    "table",
    "ul",
];

/// The elements that bound the parser's "in scope" searches; a `button` bounds the paragraph's.
const SCOPE_BOUNDS: &[&str] = &[
    "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
];

/// The elements the parser's list-item search stops at (its "special" category, less `address`,
/// `div` and `p`).
const ITEM_BOUNDS: &[&str] = &[
    "applet",
    "article",
    "aside",
    "blockquote",
    "body",
    "button",
    "caption",
    "center",
    "colgroup",
    "details",
    "dir",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "html",
    "iframe",
    "main",
    "marquee",
    "menu",
    "nav",
    "object",
    "ol",
    "pre",
    "section",
    "select",
    "summary",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
];

const HEADINGS: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];

/// Whether an ancestor in `open` (outermost first) named in `hits` is reached before one named in
/// `bounds`, searching from the innermost.
fn open_within(open: &[String], hits: &[&str], bounds: &[&str]) -> bool {
    for a in open.iter().rev() {
        if hits.contains(&a.as_str()) {
            return true;
        }
        if bounds.contains(&a.as_str()) {
            return false;
        }
    }
    false
}

/// The rewrite the HTML parser makes to `<tag>` opened with `open` (outermost first) around it.
fn rewrite_of(tag: &str, open: &[String]) -> Option<ParserRewrite> {
    use ParserRewrite::*;
    let parent = open.last().map(String::as_str).unwrap_or("");
    let table_content = |t: &str| match parent {
        "table" => matches!(
            t,
            "caption"
                | "colgroup"
                | "col"
                | "thead"
                | "tbody"
                | "tfoot"
                | "tr"
                | "script"
                | "template"
                | "style"
        ),
        "tbody" | "thead" | "tfoot" => {
            matches!(t, "tr" | "td" | "th" | "script" | "template" | "style")
        }
        "tr" => matches!(t, "td" | "th" | "script" | "template" | "style"),
        _ => true,
    };
    if parent == "table" && tag == "tr" {
        return Some(WrapsRowInTbody);
    }
    if matches!(parent, "table" | "tbody" | "thead" | "tfoot") && matches!(tag, "td" | "th") {
        return Some(WrapsCellInRow);
    }
    if parent == "table" && tag == "col" {
        return Some(WrapsColInColgroup);
    }
    if !table_content(tag) {
        return Some(MovesOutOfTable);
    }
    if CLOSES_P.contains(&tag) && open_within(open, &["p"], &[SCOPE_BOUNDS, &["button"]].concat()) {
        return Some(ClosesParagraph);
    }
    match tag {
        "a" if open_within(
            open,
            &["a"],
            &[
                "applet", "caption", "marquee", "object", "template", "td", "th",
            ],
        ) =>
        {
            Some(ClosesLink)
        }
        "form" if open.iter().any(|a| a == "form") => Some(DropsNestedForm),
        "button" if open_within(open, &["button"], SCOPE_BOUNDS) => Some(ClosesButton),
        "li" if open_within(open, &["li"], ITEM_BOUNDS) => Some(ClosesItem),
        "dd" | "dt" if open_within(open, &["dd", "dt"], ITEM_BOUNDS) => Some(ClosesItem),
        "option" if parent == "option" => Some(ClosesOption),
        "optgroup" if matches!(parent, "option" | "optgroup") => Some(ClosesOption),
        _ if HEADINGS.contains(&tag) && HEADINGS.contains(&parent) => Some(ClosesHeading),
        _ => None,
    }
}

/// The markup the HTML parser would not keep as written — see [`FindingKind::RewrittenByParser`].
/// `open` holds the elements around `nodes`, outermost first; a component's children are drawn
/// wherever its own view puts `{children}`, so their surroundings are not known here.
fn parser_rewrites(nodes: &[TNode], open: &mut Vec<String>, out: &mut Vec<Finding>) {
    for n in nodes {
        match n {
            TNode::Element {
                tag,
                children,
                span,
                ..
            } => {
                let t = tag.to_ascii_lowercase();
                if let Some(rewrite) = rewrite_of(&t, open) {
                    out.push(finding(
                        FindingKind::RewrittenByParser(rewrite, format!("`<{t}>`")),
                        *span,
                    ));
                }
                open.push(t);
                parser_rewrites(children, open, out);
                open.pop();
            }
            TNode::Text { text, span }
                if !text.trim().is_empty()
                    && TABLE_PARTS.contains(&open.last().map(String::as_str).unwrap_or("")) =>
            {
                out.push(finding(
                    FindingKind::RewrittenByParser(ParserRewrite::MovesOutOfTable, "text".into()),
                    *span,
                ));
            }
            TNode::Hole { span, .. }
                if TABLE_PARTS.contains(&open.last().map(String::as_str).unwrap_or("")) =>
            {
                out.push(finding(
                    FindingKind::RewrittenByParser(ParserRewrite::MovesOutOfTable, "text".into()),
                    *span,
                ));
            }
            TNode::Component { children, .. } => parser_rewrites(children, &mut Vec::new(), out),
            TNode::Block {
                default: Some(body),
                ..
            } => parser_rewrites(body, open, out),
            TNode::If {
                arms, otherwise, ..
            } => {
                for (_, body) in arms {
                    parser_rewrites(body, open, out);
                }
                if let Some(o) = otherwise {
                    parser_rewrites(o, open, out);
                }
            }
            TNode::For { body, empty, .. } => {
                parser_rewrites(body, open, out);
                if let Some(e) = empty {
                    parser_rewrites(e, open, out);
                }
            }
            TNode::Inspect { arms, .. } => {
                for (_, body) in arms {
                    parser_rewrites(body, open, out);
                }
            }
            _ => {}
        }
    }
}

fn node_span(n: &TNode) -> Span {
    match n {
        TNode::Element { span, .. }
        | TNode::Text { span, .. }
        | TNode::Hole { span, .. }
        | TNode::If { span, .. }
        | TNode::For { span, .. }
        | TNode::Inspect { span, .. }
        | TNode::Let { span, .. }
        | TNode::Extends { span, .. }
        | TNode::Block { span, .. }
        | TNode::Children { span }
        | TNode::Component { span, .. } => *span,
    }
}

/// Every `for` value of a `<label for="…">` anywhere in the tree (static values only — a
/// dynamic `for={…}` is trusted).
fn collect_label_targets(nodes: &[TNode]) -> Vec<String> {
    let mut out = Vec::new();
    fn go(nodes: &[TNode], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                TNode::Element {
                    tag,
                    attrs,
                    children,
                    ..
                } => {
                    if tag.eq_ignore_ascii_case("label") {
                        if let Some(v) = static_attr(attrs, "for") {
                            out.push(v.to_string());
                        }
                    }
                    go(children, out);
                }
                TNode::Component { children, .. }
                | TNode::Block {
                    default: Some(children),
                    ..
                } => go(children, out),
                TNode::If {
                    arms, otherwise, ..
                } => {
                    for (_, b) in arms {
                        go(b, out);
                    }
                    if let Some(o) = otherwise {
                        go(o, out);
                    }
                }
                TNode::For { body, empty, .. } => {
                    go(body, out);
                    if let Some(e) = empty {
                        go(e, out);
                    }
                }
                TNode::Inspect { arms, .. } => {
                    for (_, b) in arms {
                        go(b, out);
                    }
                }
                _ => {}
            }
        }
    }
    go(nodes, &mut out);
    out
}

fn static_attr<'a>(attrs: &'a [TAttr], name: &str) -> Option<&'a str> {
    attrs.iter().find_map(|a| match a {
        TAttr::Static { name: n, value, .. } if n.eq_ignore_ascii_case(name) => {
            Some(value.as_deref().unwrap_or(""))
        }
        _ => None,
    })
}

fn has_attr(attrs: &[TAttr], name: &str) -> bool {
    attrs.iter().any(|a| match a {
        TAttr::Static { name: n, .. } | TAttr::Dynamic { name: n, .. } => {
            n.eq_ignore_ascii_case(name)
        }
        _ => false,
    })
}

/// The name of an inline event-handler attribute on this element, if it carries one.
///
/// ★ THE RULE IS THE SHAPE, NOT A LIST OF EVENT NAMES. HTML defines an event-handler content
/// attribute as `on` followed by an event type, and event types are invented faster than any list
/// here could track — a denylist of `onclick`/`onerror`/`onload` is exactly the enumeration that
/// goes stale, and it fails OPEN when it does. So the test is structural: `on` + an ASCII letter.
///
/// The framework's own directive is `on-click`, whose third character is `-`, so it can never match
/// — and it arrives as `TAttr::On` rather than `Static`/`Dynamic` anyway, so it is not even
/// considered here. That is the distinction the message points at.
fn inline_handler_attr(attrs: &[TAttr]) -> Option<String> {
    attrs.iter().find_map(|a| {
        let name = match a {
            TAttr::Static { name, .. } | TAttr::Dynamic { name, .. } => name,
            _ => return None,
        };
        let lower = name.to_ascii_lowercase();
        let rest = lower.strip_prefix("on")?;
        rest.chars()
            .next()
            .filter(char::is_ascii_alphabetic)
            .map(|_| lower)
    })
}

/// Whether `method` names a submission that changes something on the server.
fn method_changes_state(method: &str) -> bool {
    matches!(
        method.to_ascii_lowercase().as_str(),
        "post" | "put" | "patch" | "delete"
    )
}

/// The method this form actually submits with, when that submission changes something — `None`
/// when it is provably a plain `get`.
///
/// ★ THIS GUARD FAILS CLOSED, and it did not used to. It read `method` with [`static_attr`], which
/// matches only a LITERAL attribute, so `<form method={m}>` answered `None`, defaulted to `"get"`,
/// and shipped with no token — a computed method silently opting out of the one check that is an
/// Error rather than a warning. An attribute the checker cannot read must mean "I cannot tell, so
/// demand the token", never "it is safe". Getting that backwards is how a defence becomes decoration.
///
/// ★ AND `formmethod` OVERRIDES THE FORM. HTML lets a submit button carry its own method, so
/// `<form action="/x"><button type="submit" formmethod="post">` is a state-changing submission on a
/// form whose own `method` says `get`. Nothing modelled that attribute at all, so the whole shape
/// was invisible. The subtree is walked because the button may sit inside a `{if}`, a fieldset or a
/// row, exactly as [`has_csrf_field`] walks it.
fn state_changing_method(attrs: &[TAttr], children: &[TNode]) -> Option<String> {
    // The form's own method: a literal decides; anything computed is unreadable and so counts.
    match static_attr(attrs, "method") {
        Some(m) if method_changes_state(m) => return Some(m.to_ascii_lowercase()),
        Some(_) => {}
        None if has_attr(attrs, "method") => return Some("post".to_string()),
        None => {}
    }
    overriding_method(children)
}

/// A `formmethod` anywhere under this form that changes state — or one computed, which cannot be
/// read and therefore counts.
fn overriding_method(nodes: &[TNode]) -> Option<String> {
    nodes.iter().find_map(|n| match n {
        TNode::Element {
            attrs, children, ..
        } => match static_attr(attrs, "formmethod") {
            Some(m) if method_changes_state(m) => Some(m.to_ascii_lowercase()),
            Some(_) => overriding_method(children),
            None if has_attr(attrs, "formmethod") => Some("post".to_string()),
            None => overriding_method(children),
        },
        TNode::If {
            arms, otherwise, ..
        } => arms
            .iter()
            .find_map(|(_, b)| overriding_method(b))
            .or_else(|| otherwise.as_deref().and_then(overriding_method)),
        TNode::For { body, empty, .. } => {
            overriding_method(body).or_else(|| empty.as_deref().and_then(overriding_method))
        }
        TNode::Inspect { arms, .. } => arms.iter().find_map(|(_, b)| overriding_method(b)),
        TNode::Block {
            default: Some(body),
            ..
        } => overriding_method(body),
        TNode::Component { children, .. } => overriding_method(children),
        _ => None,
    })
}

fn has_visible_content(children: &[TNode]) -> bool {
    children.iter().any(|c| match c {
        TNode::Text { text, .. } => !text.trim().is_empty(),
        TNode::Hole { .. }
        | TNode::Component { .. }
        | TNode::Children { .. }
        | TNode::Block { .. } => true,
        TNode::Element {
            tag,
            attrs,
            children,
            ..
        } => {
            (tag.eq_ignore_ascii_case("img")
                && static_attr(attrs, "alt").is_some_and(|a| !a.is_empty()))
                || has_visible_content(children)
        }
        TNode::If {
            arms, otherwise, ..
        } => {
            arms.iter().any(|(_, b)| has_visible_content(b))
                || otherwise.as_deref().is_some_and(has_visible_content)
        }
        TNode::For { body, .. } => has_visible_content(body),
        TNode::Inspect { arms, .. } => arms.iter().any(|(_, b)| has_visible_content(b)),
        _ => false,
    })
}

fn walk(
    nodes: &[TNode],
    inside_label: bool,
    labels_for: &[String],
    blocks: &mut Vec<String>,
    children_seen: &mut usize,
    out: &mut Vec<Finding>,
) {
    for n in nodes {
        match n {
            TNode::Element {
                tag,
                attrs,
                children,
                span,
            } => {
                let t = tag.to_ascii_lowercase();
                if attrs.iter().any(|a| matches!(a, TAttr::Bind { .. }))
                    && !matches!(t.as_str(), "input" | "textarea" | "select")
                {
                    out.push(finding(FindingKind::BindOnNonInput(tag.clone()), *span));
                }
                // A `bind` on a control the browser will not let a page write. See
                // `FindingKind::BindNotHonoured`; the two elements are told apart by TAG and by a
                // boolean attribute, neither of which the attribute-only question could see.
                if attrs.iter().any(|a| matches!(a, TAttr::Bind { .. })) {
                    let refused = if t == "select" && has_attr(attrs, "multiple") {
                        crate::ast::view::BoundInput::SelectMultiple.refusal()
                    } else if t == "input"
                        && static_attr(attrs, "type")
                            .is_some_and(|v| v.eq_ignore_ascii_case("file"))
                    {
                        crate::ast::view::BoundInput::File.refusal()
                    } else {
                        None
                    };
                    if let Some(why) = refused {
                        out.push(finding(FindingKind::BindNotHonoured(why), *span));
                    }
                }
                if let Some(handler) = inline_handler_attr(attrs) {
                    out.push(finding(FindingKind::InlineEventHandler(handler), *span));
                }
                // A form whose method CHANGES something must carry the token. Checked on the
                // form's own subtree, so a form nested in a card or a list item is found — a
                // check that only looked at the top level would pass every real page.
                if t == "form" {
                    if let Some(method) = state_changing_method(attrs, children) {
                        if !has_csrf_field(children) {
                            out.push(finding(FindingKind::FormWithoutCsrf(method), *span));
                        }
                    }
                }
                match t.as_str() {
                    "img" if !has_attr(attrs, "alt") => {
                        out.push(finding(FindingKind::ImageWithoutAlt, *span))
                    }
                    "button"
                        if !has_visible_content(children)
                            && !has_attr(attrs, "aria-label")
                            && !has_attr(attrs, "aria-labelledby")
                            && !has_attr(attrs, "title") =>
                    {
                        out.push(finding(FindingKind::ButtonWithoutName, *span))
                    }
                    "input" | "textarea" | "select" => {
                        let ty = static_attr(attrs, "type")
                            .unwrap_or("text")
                            .to_ascii_lowercase();
                        let needs_label = t != "input"
                            || !matches!(
                                ty.as_str(),
                                "hidden" | "submit" | "button" | "reset" | "image"
                            );
                        let labelled = inside_label
                            || has_attr(attrs, "aria-label")
                            || has_attr(attrs, "aria-labelledby")
                            || attrs.iter().any(|a| matches!(a, TAttr::Dynamic { name, .. } if name.eq_ignore_ascii_case("id")))
                            || static_attr(attrs, "id").is_some_and(|id| labels_for.iter().any(|l| l == id));
                        if needs_label && !labelled {
                            out.push(finding(FindingKind::InputWithoutLabel, *span));
                        }
                    }
                    "a" if !has_attr(attrs, "href") => {
                        out.push(finding(FindingKind::LinkWithoutHref, *span))
                    }
                    _ => {}
                }
                walk(
                    children,
                    inside_label || t == "label",
                    labels_for,
                    blocks,
                    children_seen,
                    out,
                );
            }
            TNode::Component {
                name,
                props,
                children,
                span,
            } => {
                if props.iter().any(|a| matches!(a, TAttr::Class { .. })) {
                    out.push(finding(
                        FindingKind::ClassToggleOnComponent(name.clone()),
                        *span,
                    ));
                }
                walk(
                    children,
                    inside_label,
                    labels_for,
                    blocks,
                    children_seen,
                    out,
                );
            }
            TNode::Block {
                name,
                default,
                span,
            } => {
                if blocks.iter().any(|b| b == name) {
                    out.push(finding(FindingKind::DuplicateBlock(name.clone()), *span));
                } else {
                    blocks.push(name.clone());
                }
                if let Some(d) = default {
                    walk(d, inside_label, labels_for, blocks, children_seen, out);
                }
            }
            TNode::Children { span } => {
                *children_seen += 1;
                if *children_seen > 1 {
                    out.push(finding(FindingKind::DuplicateChildren, *span));
                }
            }
            TNode::If {
                arms, otherwise, ..
            } => {
                for (_, b) in arms {
                    walk(b, inside_label, labels_for, blocks, children_seen, out);
                }
                if let Some(o) = otherwise {
                    walk(o, inside_label, labels_for, blocks, children_seen, out);
                }
            }
            TNode::For { body, empty, .. } => {
                walk(body, inside_label, labels_for, blocks, children_seen, out);
                if let Some(e) = empty {
                    walk(e, inside_label, labels_for, blocks, children_seen, out);
                }
            }
            TNode::Inspect { arms, .. } => {
                for (_, b) in arms {
                    walk(b, inside_label, labels_for, blocks, children_seen, out);
                }
            }
            TNode::Text { .. } | TNode::Hole { .. } | TNode::Let { .. } | TNode::Extends { .. } => {
            }
        }
    }
}

/// The elements a document's head may hold. `<style>` and `<script>` are refused by
/// the tokenizer before this ever runs, and `<base>` is included because a site
/// served under a path prefix needs it.
///
/// ★ `<html>` IS HERE AND IS NOT A HEAD ELEMENT, DELIBERATELY. `<html lang="fr">` in a `Head:`
/// block is how a page declares its language — `lang` is what a screen reader picks a voice from
/// and what a browser offers to translate on, and it was welded to `en` for every page LOGOS has
/// ever built. The document builder LIFTS the tag onto the document element rather than rendering
/// it, so it never reaches the head it is written in; allowing it here is what lets it be written
/// at all. Anything else genuinely misplaced — a `<div>`, a `<p>` — is still caught.
const HEAD_ELEMENTS: &[&str] = &["title", "meta", "link", "base", "html"];

/// The first element in `nodes` that does not belong in a document's head, with its
/// span — `None` when every element is head-legal.
///
/// A browser relocates a body element out of the head rather than reporting it, so
/// this is the only place the mistake can be caught where it was made.
pub fn first_non_head_element(
    nodes: &[crate::lgx::template::TNode],
) -> Option<(String, crate::token::Span)> {
    use crate::lgx::template::TNode;
    for n in nodes {
        match n {
            TNode::Element {
                tag,
                children,
                span,
                ..
            } => {
                if !HEAD_ELEMENTS.contains(&tag.to_ascii_lowercase().as_str()) {
                    return Some((tag.clone(), *span));
                }
                // `<title>Hello {who}</title>` has children; they are text and holes,
                // never elements, but walking is what makes that a fact rather than
                // an assumption.
                if let Some(bad) = first_non_head_element(children) {
                    return Some(bad);
                }
            }
            TNode::Component { name, span, .. } => return Some((name.clone(), *span)),
            _ => {}
        }
    }
    None
}
