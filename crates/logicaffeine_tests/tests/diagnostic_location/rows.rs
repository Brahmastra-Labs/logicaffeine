//! The mistakes `diagnostic_location_lock` holds every surface to: one row per member of each
//! class of LGX diagnostic, each a page whose mistake sits on line 3 or later, and where it sits.
//!
//! Included by the compiler's lock (`crates/logicaffeine_tests/tests/diagnostic_location_lock.rs`)
//! and the editor's (`crates/logicaffeine_tests/tests/diagnostic_location_editor_lock.rs`), so the
//! two check one table.
#![allow(dead_code)]

/// Which member of which class of diagnostic a row exercises — the name the census matches
/// against the SOURCE of that class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    /// A `NotLowerable::at(…)` call in `parser/lgx_lower.rs`: `(enclosing fn, message key)`. The
    /// key is the first four words of the message literal, or the argument's name when the message
    /// is forwarded from elsewhere (`why`).
    NotLowerable(&'static str, &'static str),
    /// A `lgx::check::FindingKind` variant.
    Finding(&'static str),
    /// A `lgx::template::TemplateErrorKind` variant other than `Html`.
    Template(&'static str),
    /// A `lgx::html::HtmlErrorKind` variant (reached through `TemplateErrorKind::Html`).
    Html(&'static str),
    /// A `lgx::html::ModifierError` variant (reached through `TemplateErrorKind::EventModifier`).
    Modifier(&'static str),
    /// A `lgx::css::CssErrorKind` variant.
    Css(&'static str),
    /// A name-resolution or structure check of the LGX parser (`parser/component.rs`):
    /// `(fn, message key)`, keyed like `NotLowerable`.
    Check(&'static str, &'static str),
    /// A `lint::FindingKind` variant — the value-semantics copy lint, a WARNING never reached
    /// through the browser build's own refusal path (it never refuses anything), so its own
    /// surface check (`largo_check_reports_every_warning_where_it_is`) reads
    /// `value_copy_findings_of` for this member instead of the template checker's findings.
    CopyLint(&'static str),
}

/// Where a diagnostic reaches the author.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// `compile_to_wasm` — the browser build `largo build` and `largo serve` compile through; its
    /// `Err` carries the span. The tree-walker parses through the same seam, so a PARSE error's
    /// location here is its location on every tier.
    Browser,
    /// `ui_bridge::lgx_findings_of` — the template checker's warnings as `largo check` reads them.
    Check,
    /// The language server's published diagnostic.
    Editor,
}

/// How severe the member is, which decides the surfaces it reaches: an error stops the build (the
/// browser build and the editor locate it); a warning never does (`largo check` and the editor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Whether a program can reach the member at all.
#[derive(Clone, Debug)]
pub enum Reach {
    /// This page does: its mistake is the member's.
    Page {
        source: String,
        /// The construct the diagnostic must point INTO: its first occurrence in `source`.
        at: &'static str,
        /// A phrase the diagnostic's message carries, so the row measures its own member rather
        /// than an unrelated refusal of the same page.
        says: &'static str,
        severity: Severity,
        /// Surfaces that do not locate this row yet — the diagnostic is elsewhere, or not published
        /// at all — each with the request it waits on. A pending row that starts locating correctly
        /// FAILS the lock until it is removed from here.
        pending: &'static [(Surface, &'static str)],
    },
    /// No program reaches it today, and why. Listed so a census over the source stays total.
    Unreachable(&'static str),
}

#[derive(Clone, Debug)]
pub struct Row {
    pub member: Member,
    pub reach: Reach,
}

/// The editor's per-block recovery reparses each `##` block as its own mini-program (lang L11);
/// a check that is only reachable with cross-block knowledge (which name is a store, which name
/// is an action) sees nothing to complain about in EITHER block on its own, so the full-file
/// diagnosis — landing on a line no isolated block's own reparse also flags — is never republished.
/// Distinct from `SPURIOUS_WHILE_L11` in `diagnostic_location_editor_lock.rs` (an unwanted EXTRA
/// diagnostic): this is the same root cause producing a MISSING one.
pub const L11_CROSS_BLOCK: &str =
    "lang L11: a fragment parse for the editor's block recovery (cross-block checks are invisible \
     to a single block's own reparse)";

/// A component whose `View:` is `view` (each line indented under `View:`), mounted by `## Main`.
/// The view's first line is line 10.
pub fn component(view: &str) -> String {
    format!(
        "## Component App\n\
         State:\n\
        \x20   Let n be 0.\n\
        \x20   Let draft be \"\".\n\
        \x20   Let kind be \"text\".\n\
         Actions:\n\
        \x20   To go:\n\
        \x20       Set n to n + 1.\n\
         View:\n\
         {}\n\n\
         ## Main\n\
         Mount App.\n",
        indent(view)
    )
}

/// [`component`] with `sentences` added to its `Actions:` (each line indented under `Actions:`).
/// The first added sentence is on line 9.
pub fn component_acting(sentences: &str, view: &str) -> String {
    component(view).replace(
        "        Set n to n + 1.\nView:",
        &format!("        Set n to n + 1.\n{}\nView:", indent(sentences)),
    )
}

/// An `## Error page 404` whose view is `view`, beside a `## Main`. The view's first line is line 7.
pub fn error_page(view: &str) -> String {
    format!(
        "# Server\n\n\
         ## Error page 404\n\
         State:\n\
        \x20   Let h be \"<b>hi</b>\".\n\
         View:\n\
         {}\n\n\
         ## Main\n\
         Show \"ok\".\n",
        indent(view)
    )
}

fn indent(view: &str) -> String {
    view.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn page(source: String, at: &'static str, says: &'static str) -> Reach {
    Reach::Page {
        source,
        at,
        says,
        severity: Severity::Error,
        pending: &[],
    }
}

fn pending(
    source: String,
    at: &'static str,
    says: &'static str,
    pending: &'static [(Surface, &'static str)],
) -> Reach {
    Reach::Page {
        source,
        at,
        says,
        severity: Severity::Error,
        pending,
    }
}

fn warning(source: String, at: &'static str, says: &'static str) -> Reach {
    Reach::Page {
        source,
        at,
        says,
        severity: Severity::Warning,
        pending: &[],
    }
}

/// Every row.
pub fn rows() -> Vec<Row> {
    use Member::*;
    let r = |member: Member, reach: Reach| Row { member, reach };
    vec![
        // ── NotLowerable: the server-side walk (`## Error page`, `-> View` routes) ──────────────
        r(
            NotLowerable("view_html_lowerable_seen", "why"),
            page(
                error_page("<form>\n  <input type={h} bind={h} aria-label=\"x\">\n</form>"),
                "<input type={h}",
                "is computed",
            ),
        ),
        r(
            NotLowerable("view_html_lowerable_seen", "its template uses a"),
            page(
                format!(
                    "## Directive tooltip (el: Element) and (text: Text):\n    Call setAttribute with \
                     el and \"title\" and text.\n\n{}",
                    error_page("<p tooltip={h}>x</p>")
                ),
                "<p tooltip",
                "uses a directive",
            ),
        ),
        r(
            NotLowerable("view_html_lowerable_seen", "its template places `<{}/>`,"),
            page(
                format!(
                    "## Component Card\nView:\n    <p>card</p>\n\n{}",
                    error_page("<div>\n  <Card/>\n</div>")
                ),
                "<Card/>",
                "cannot see",
            ),
        ),
        r(
            NotLowerable("view_html_lowerable_seen", "its template places `<{}/>`,#2"),
            page(
                "# Server\n\n## Component Empty\nState:\n    Let n be 0.\n\n\
                 ## Route GET /report -> View:\n    <div>\n      <Empty/>\n    </div>\n\n\
                 ## Main\nShow \"ok\".\n"
                    .to_string(),
                "<Empty/>",
                "no `View:` to render",
            ),
        ),
        r(
            NotLowerable("view_html_lowerable_seen", "its template uses {},"),
            page(error_page("<div>{raw h}</div>"), "{raw h}", "does not cover"),
        ),
        // ── NotLowerable: routes ──────────────────────────────────────────────────────────────
        r(
            NotLowerable("route_lowerable", "one of its parameters"),
            page(
                "# Server\n\n## Route GET /api/todos -> Text:\n    Return \"all\".\n\n\
                 ## Route GET /api/todos/:flag (flag: Bool) -> Text:\n    Return \"x\".\n\n\
                 ## Main\nShow \"ok\".\n"
                    .to_string(),
                "## Route GET /api/todos/:flag",
                "path matcher cannot bind",
            ),
        ),
        r(
            NotLowerable("route_lowerable", "it answers with a"),
            page(
                "# Server\n\n## Route GET /api/count -> Text:\n    Return \"1\".\n\n\
                 ## Route GET /api/pair -> Seq of Int:\n    Return [1, 2].\n\n## Main\nShow \"ok\".\n"
                    .to_string(),
                "## Route GET /api/pair",
                "derived JSON codecs",
            ),
        ),
        // ── NotLowerable: a mounted component ─────────────────────────────────────────────────
        r(
            NotLowerable("lowerable", "a store or error"),
            Reach::Unreachable(
                "a `## Store` or `## Error page` placed in a view is refused earlier by the \
                 resolver's `resolve_placement` (kind check), and named in a top-level `Mount` is \
                 refused earlier still by `check_mounts_name_something` (stores and error pages are \
                 not in `self.components`) — no program that reaches `lowerable` still has one",
            ),
        ),
        r(
            NotLowerable("callback_binding_refusal", "`{p}` of `<{tag}>` is"),
            // A top-level `## To` function is a callback SHAPE to the earlier resolver pass
            // (`lgx_resolve.rs::shape_of`'s `Stmt::FunctionDef` arm), so it clears that check and
            // reaches this one — where it is neither an action of the placer nor a forwarded prop.
            page(
                format!(
                    "## To ignored () -> Unit:\n    Show \"x\".\n\n\
                     ## Component Child (onGo: fn() -> Unit)\nView:\n    <button on-click={{onGo}}>go</button>\n\n{}",
                    component("<div>\n  <Child onGo={ignored}/>\n</div>")
                ),
                "<Child onGo",
                "neither one of this component's actions",
            ),
        ),
        r(
            NotLowerable("callback_binding_refusal", "`{p}` of `<{tag}>` is#2"),
            page(
                format!(
                    "## Component Child (onGo: fn(Text) -> Unit)\nView:\n    <button on-click={{onGo(\"x\")}}>go</button>\n\n{}",
                    component("<div>\n  <Child onGo={go}/>\n</div>")
                ),
                "<Child onGo",
                "takes another",
            ),
        ),
        r(
            NotLowerable("lowerable", "its action `{}` reaches"),
            page(
                "# App\n\n## Component App\nState:\n    Let n be 0.\nActions:\n    To ping:\n        \
                 Call pong.\n    To pong:\n        Call ping.\nView:\n    <p>{n}</p>\n\n## Main\nMount App.\n"
                    .to_string(),
                "To ping:",
                "reaches itself",
            ),
        ),
        // A `<Tag/>` placement naming a real but non-`## Component` def (a store, a page, an
        // error page) is refused by the resolver, at the placement — before `lowerable` (above)
        // ever sees it; `lowerable`'s own text is reached only through a top-level `Mount`, which
        // the resolver does not check.
        r(
            Check("resolve_placement", "`<{tag}>` places {label}, which"),
            page(
                "# Store\n\n## Store Session\nState:\n    Let user be \"\".\n\n## Component App\n\
                 View:\n    <div>\n      <Session/>\n    </div>\n\n## Main\nMount App.\n"
                    .to_string(),
                "<Session/>",
                "not a component a view can place",
            ),
        ),
        r(
            Check("resolve_placement", "`<{tag}/>` places `{tag}`, which"),
            page(
                "# Card\n\n## Component Card\nHead:\n    <title>Card</title>\nView:\n    <p>card</p>\n\n\
                 ## Component App\nView:\n    <div>\n      <Card/>\n    </div>\n\n## Main\nMount App.\n"
                    .to_string(),
                "<Card/>",
                "carries a `Head:`",
            ),
        ),
        r(
            NotLowerable("lowerable", "why"),
            page(
                "# Store\n\n## Store Prefs persisted as \"prefs\"\nState:\n    Let tags be [\"a\"].\n\n\
                 ## Component App\nUse Prefs.\nView:\n    <p>x</p>\n\n## Main\nMount App.\n"
                    .to_string(),
                "## Store Prefs",
                "cannot give a record type",
            ),
        ),
        r(
            NotLowerable("lowerable", "why#2"),
            page(
                component_acting(
                    "To load:\n    Get \"/api/x\" as Real, then loaded.\nTo loaded (r: Result of Real and HttpError):\n    Set n to 0.",
                    "<div>\n  <p>{n}</p>\n</div>",
                ),
                "To load:",
                "is not a type this fetch can read yet",
            ),
        ),
        r(
            NotLowerable("lowerable", "it uses `{0}`, and"),
            Reach::Unreachable(
                "the parser's `resolve_uses` refuses `Use Nowhere.` at the sentence, over the same \
                 whole-program store list this check reads — no program that reaches `lowerable` can \
                 still be using an undeclared store",
            ),
        ),
        r(
            NotLowerable("lowerable", "its `Style:` block is"),
            page(
                "# App\n\n## Component App\nView:\n    <p>x</p>\nStyle:\n    p { color: red; }\n    \
                 em { color: blue;\n\n## Main\nMount App.\n"
                    .to_string(),
                "{ color: blue;",
                "not valid CSS",
            ),
        ),
        r(
            NotLowerable("lowerable", "its `{}` sentence waits"),
            Reach::Unreachable(
                "every `Trigger` variant is in the allow-list `lowerable` checks before this \
                 refusal: no sentence a component can write waits on something the browser tier \
                 cannot deliver",
            ),
        ),
        r(
            NotLowerable("lowerable", "it has no `View:`"),
            page(
                "# App\n\n## Component App\nState:\n    Let n be 0.\n\n## Main\nMount App.\n"
                    .to_string(),
                "## Component App",
                "no `View:`",
            ),
        ),
        // ── NotLowerable: the browser walk over a view ────────────────────────────────────────
        r(
            NotLowerable("nodes_lowerable", "why"),
            page(component("<div>\n  <input type={kind} bind={draft} aria-label=\"x\">\n</div>"), "<input type={kind}", "is computed"),
        ),
        r(
            NotLowerable("nodes_lowerable", "the layout it extends,"),
            page(
                format!(
                    "## Component Site (title: Text)\nView:\n    <main>{{title}}{{block content}}</main>\n\n{}",
                    component("{extends Site}\n{block content:}<p>hi</p>{end}")
                ),
                "{extends Site}",
                "declares props",
            ),
        ),
        r(
            NotLowerable("nodes_lowerable", "it extends `{0}`, and"),
            Reach::Unreachable(
                "the resolver's `unknown_component` refuses `{extends Nowhere}` at parse time, over \
                 the same whole-program component list this check reads — no program that reaches \
                 `nodes_lowerable` can still be extending an undeclared layout",
            ),
        ),
        r(
            NotLowerable("nodes_lowerable", "it places `<{0}/>`, and"),
            Reach::Unreachable(
                "the resolver's `unknown_component` refuses `<Nowhere/>` at parse time, over the same \
                 whole-program component list this check reads — no program that reaches \
                 `nodes_lowerable` can still be placing an undeclared component",
            ),
        ),
        // ── The template checker's findings ───────────────────────────────────────────────────
        r(
            Finding("ContentOutsideBlock"),
            page(
                format!(
                    "## Component Site\nView:\n    <main>{{block content}}</main>\n\n{}",
                    component("{extends Site}\n<p>loose</p>\n{block content:}<p>hi</p>{end}")
                ),
                "<p>loose</p>",
                "fills the layout's blocks",
            ),
        ),
        r(
            Finding("DuplicateBlock"),
            page(component("<main>\n  {block a:}<p>1</p>{end}\n  {block a:}<p>2</p>{end}\n</main>"), "{block a:}<p>2", "defined twice"),
        ),
        r(
            Finding("DuplicateChildren"),
            page(component("<div>{children}</div>\n<p>{children}</p>"), "{children}</p>", "can appear once"),
        ),
        r(Finding("BindOnNonInput"), page(component("<div>\n  <p bind={draft}>x</p>\n</div>"), "<p bind", "has none")),
        r(
            Finding("ClassToggleOnComponent"),
            page(
                format!("## Component Card\nView:\n    <p>c</p>\n\n{}", component("<div>\n  <Card class:big={n}/>\n</div>")),
                "<Card class:big",
                "takes props",
            ),
        ),
        r(Finding("ImageWithoutAlt"), warning(component("<div>\n  <img src=\"cat.png\">\n</div>"), "<img", "no `alt`")),
        r(
            Finding("ButtonWithoutName"),
            warning(component("<div>\n  <button on-click={go}></button>\n</div>"), "<button", "announces nothing"),
        ),
        r(
            Finding("InputWithoutLabel"),
            warning(component("<div>\n  <input type=\"text\" bind={draft}>\n</div>"), "<input", "no label"),
        ),
        r(Finding("LinkWithoutHref"), warning(component("<div>\n  <a>home</a>\n</div>"), "<a>", "no `href`")),
        r(
            Finding("FormWithoutCsrf"),
            page(component("<div>\n  <form method=\"post\"><button>go</button></form>\n</div>"), "<form", "changes something"),
        ),
        r(
            Finding("InlineEventHandler"),
            page(component("<div>\n  <button onclick=\"go()\">go</button>\n</div>"), "<button onclick", "inline JavaScript"),
        ),
        r(
            Finding("BindNotHonoured"),
            page(component("<div>\n  <input type=\"file\" bind={draft} aria-label=\"f\">\n</div>"), "<input type=\"file\"", "file input"),
        ),
        r(
            Finding("RewrittenByParser"),
            warning(component("<table>\n  <tr><td>x</td></tr>\n</table>"), "<tr>", "wraps the row in a `<tbody>`"),
        ),
        // ── Template errors ───────────────────────────────────────────────────────────────────
        r(Template("UnexpectedEnd"), page(component("<div>\n  <p>x</p>{end}\n</div>"), "{end}", "closes nothing")),
        r(Template("UnclosedTag"), page(component("<div>\n  {if n:}<p>x</p>\n</div>"), "{if n:}", "never closed")),
        r(Template("MismatchedEndTag"), page(component("<div>\n  <p>x</div>"), "</div>", "closes the wrong element")),
        r(Template("UnclosedElement"), page(component("<section>\n  <p>x</p>"), "<section>", "never closed")),
        r(Template("StrayEndTag"), page(component("<div>x</div>\n</span>"), "</span>", "closes nothing")),
        r(Template("UnexpectedEndTagOfVoid"), page(component("<div>\n  <br></br>\n</div>"), "</br>", "void element")),
        r(Template("ExtendsNotFirst"), page(component("<p>x</p>\n{extends Site}"), "{extends Site}", "very first thing")),
        r(Template("StrayElse"), page(component("<div>\n  {else:}\n</div>"), "{else:}", "no `{if")),
        r(
            Template("DuplicateElse"),
            page(component("<div>\n  {if n:}a{else:}b{else:}c{end}\n</div>"), "{else:}c", "only one `{else:}`"),
        ),
        r(Template("StrayWhen"), page(component("<div>\n  {when Ok:}x\n</div>"), "{when Ok:}", "only belongs inside")),
        r(Template("TagLikeHole"), page(component("<div>\n  {iff n:}\n</div>"), "{iff n:}", "like a template tag")),
        r(Template("MalformedTag"), page(component("<div>\n  {if:}x{end}\n</div>"), "{if:}", "missing a part")),
        r(Template("EmptyHole"), page(component("<div>\n  <p>{}</p>\n</div>"), "{}", "is empty")),
        r(
            Template("MalformedDirective"),
            page(component("<div>\n  <button on-click=\"go\">b</button>\n</div>"), "on-click=\"go\"", "needs a `{…}` value"),
        ),
        r(
            Template("NonCanonicalTag"),
            page(component("<div>\n  {if n:}a{elif n:}b{end}\n</div>"), "{elif n:}", "is written"),
        ),
        // The retired `Render:` dialect's element and stylesheet lines, wherever a view holds them:
        // inside an element, where they would otherwise read as literal text, and as the view's
        // first line, where they would otherwise never reach the template at all.
        r(
            Template("RetiredViewForm"),
            page(component("<div>\n  div: card(\"Hello\")\n</div>"), "div: card(\"Hello\")", "retired `Render:`"),
        ),
        r(
            Template("RetiredViewForm"),
            page(component("<div>\n  css \"p { color: red }\"\n</div>"), "css \"p { color: red }\"", "`Style:` section"),
        ),
        r(
            Template("RetiredViewForm"),
            page(component("div: card(\"Hello\")\n<p>{n}</p>"), "div: card(\"Hello\")", "retired `Render:`"),
        ),
        // ── Event-modifier errors (`TemplateErrorKind::EventModifier`) ────────────────────────
        r(
            Modifier("Unknown"),
            page(component("<div>\n  <button on-click.prevnt={go}>b</button>\n</div>"), ".prevnt", "not an event qualifier"),
        ),
        r(
            Modifier("KeyOnNonKeyEvent"),
            page(component("<div>\n  <button on-click.enter={go}>b</button>\n</div>"), ".enter", "not a key event"),
        ),
        r(
            Modifier("TwoKeys"),
            page(component("<div>\n  <input on-keydown.enter.escape={go} aria-label=\"x\">\n</div>"), ".escape", "both name a key"),
        ),
        r(
            Modifier("SystemKeyNotDelivered"),
            page(component("<div>\n  <input on-keydown.ctrl={go} aria-label=\"x\">\n</div>"), ".ctrl", "modifier key"),
        ),
        r(
            Modifier("PreventOnPassive"),
            page(component("<div>\n  <input on-keydown.prevent.passive={go} aria-label=\"x\">\n</div>"), ".passive", "contradict each other"),
        ),
        // ── HTML errors (`TemplateErrorKind::Html`) ───────────────────────────────────────────
        r(Html("UnterminatedTag"), page(component("<div>\n  <p class=\"x\""), "<p class", "never closes")),
        r(
            Html("UnterminatedAttributeValue"),
            page(component("<div>\n  <p class=\"x>hi</p>\n</div>"), "\"x>hi", "never closes"),
        ),
        r(Html("UnterminatedBrace"), page(component("<div>\n  <p>{n</p>\n</div>"), "{n</p>", "never closes")),
        r(Html("StrayCloseBrace"), page(component("<div>\n  <p>}</p>\n</div>"), "}</p>", "closes nothing")),
        r(Html("UnterminatedComment"), page(component("<div>\n  <!-- note\n</div>"), "<!--", "never reaches")),
        r(Html("ForbiddenElement"), page(component("<div>\n  <script>x</script>\n</div>"), "<script", "`<script>`")),
        r(Html("BadAttributeName"), page(component("<div>\n  <p =x>hi</p>\n</div>"), "=x>", "cannot start an attribute")),
        r(Html("BadTagName"), page(component("<div>\n  <1>hi\n</div>"), "<1>", "must start a tag name")),
        // ── CSS errors ────────────────────────────────────────────────────────────────────────
        r(
            Css("UnbalancedBraces"),
            page(
                "# App\n\n## Component App\nView:\n    <p>x</p>\nStyle:\n    p { color: red; }\n    \
                 em { color: blue;\n\n## Main\nMount App.\n"
                    .to_string(),
                "{ color: blue;",
                "do not balance",
            ),
        ),
        r(
            Css("UnterminatedString"),
            page(
                "# App\n\n## Component App\nView:\n    <p>x</p>\nStyle:\n    p { color: red; }\n    \
                 em { content: \"x; }\n\n## Main\nMount App.\n"
                    .to_string(),
                "\"x; }",
                "never closes",
            ),
        ),
        // ── The LGX parser's resolution and structure checks ──────────────────────────────────
        r(
            Check("view_body_is_not_markup", "A `View:` section holds"),
            page(component("Hello there\n<p>{n}</p>"), "Hello there", "its first line must start with a tag"),
        ),
        r(
            Check("head_from_raw", "`<{tag}>` cannot go in"),
            page(
                "## Page /about\nHead:\n    <title>About</title>\n    <div>oops</div>\nView:\n    <p>x</p>\n"
                    .to_string(),
                "<div>oops",
                "cannot go in a `Head:`",
            ),
        ),
        r(
            Check("lower_view_functions", "`## To {name}`'s template"),
            pending(
                format!(
                    "# Page\n\n## To badge (label: Text) -> View:\n    <button on-click={{go}}>{{label}}</button>\n\n{}",
                    component("<p>{badge(\"x\")}</p>")
                ),
                "on-click={go}",
                "template holds an event handler",
                &[(
                    Surface::Editor,
                    "the editor drops a whole-file parse error from a file holding `## To` blocks \
                     when its per-block recovery finds none (logicaffeine_lsp pipeline.rs \
                     `has_function_blocks`), and the recovery's fragments skip `resolve_lgx`",
                )],
            ),
        ),
        r(
            Check("lower_view_functions", "why"),
            Reach::Unreachable(
                "discovery registers a template's `__View` variant by reading its header with the \
                 grammar the parser's own function header uses — the name, `(name: Type)` groups \
                 joined by `and`, a preposition, commas or nothing, `-> View:` and a markup body — so \
                 every template the parser accepts already has a variant with its parameters; the \
                 check guards the two readers against drifting apart (probed: generic, adjacent and \
                 comma groups, no parameters, `Seq of View`, record and map types, parameters named \
                 by keywords, articles and verbs)",
            ),
        ),
        r(
            Check("view_node_from_template", "`<{name}>` is a component,"),
            page(
                format!("## Component Card\nView:\n    <p>c</p>\n\n{}", component("<div>\n  <Card bind={draft}/>\n</div>")),
                "<Card bind",
                "is a component",
            ),
        ),
        r(
            Check("parse_expr_snippet", "`{}` is not one"),
            page(component("<div>\n  <p>{n 5}</p>\n</div>"), "{n 5}", "not one LOGOS expression"),
        ),
        r(
            Check("pattern_from_text", "a loop needs an"),
            page(component("<ul>\n  {for each () in n:}<li>x</li>{end}\n</ul>"), "{for each ()", "a loop needs an item name"),
        ),
        r(
            Check("pattern_from_text", "`{t}` is not an"),
            page(component("<ul>\n  {for each my item in n:}<li>x</li>{end}\n</ul>"), "{for each my item", "is not an item name"),
        ),
        r(
            Check("attrs_from_template", "`on-{event}={{…}}` names an action"),
            page(component("<div>\n  <button on-click={1 + 2}>b</button>\n</div>"), "on-click={1 + 2}", "names an action to run"),
        ),
        r(
            Check("lower_session", "`## Session` carries `{field}`,"),
            // `lower_session` reports at the field's own span (lang2:
            // `TypeRegistry::field_span`, threaded from `DiscoveryPass::scan_session_block`), and
            // the checked seam's own double-mapping bug that used to erase it on the way out
            // through `compile_to_wasm` is fixed (`with_parsed_program_checked` translates once,
            // at its own exit) — confirmed via `logicaffeine-lsp::diagnostic_location_editor_lock`:
            // the Editor surface now locates this fixture correctly too (diag2's own report).
            page(
                "# App\n\n## Session\n    a cart, which is Seq of Int.\n\n## Main\nShow \"ok\".\n".to_string(),
                "a cart",
                "a cookie cannot hold",
            ),
        ),
        r(
            Check("lower_lgx_components", "`{component}` has derived values"),
            page(
                "# App\n\n## Component App\nState:\n    Let a always be b + 1.\n    Let b always be a + 1.\n\
                 View:\n    <p>{a}</p>\n\n## Main\nMount App.\n"
                    .to_string(),
                "Let a always",
                "in a circle",
            ),
        ),
        r(
            Check("lower_component_serve", "`html of {component}` draws"),
            page(
                "# App\n\n## Component Row (label: Text)\nView:\n    <li>{label}</li>\n\n\
                 ## Main\nShow html of Row.\n"
                    .to_string(),
                "## Component Row",
                "only a placement hands it",
            ),
        ),
        r(
            Check("refuse_serving", "`html of {name}` asks"),
            // The checked seam's double-mapping bug (see the `lower_session` row above) was what
            // erased this row's Browser location too — fixed the same way, confirmed GREEN.
            page(
                "# App\n\n## Component App\nState:\n    Let h be \"<b>hi</b>\".\nView:\n    <div>{raw h}</div>\n\n\
                 ## Main\nShow html of App.\n"
                    .to_string(),
                "{raw h}",
                "server-rendered markup",
            ),
        ),
        r(
            Check("check_fetches_in", "this page fetches `{path}`"),
            // `check_wire_contract` builds this error with the enclosing handler's own span
            // (`e.span`, verified at its construction site in `parse_actions_section`,
            // `component.rs:907`) — L5 landed, and the checked seam's double-mapping bug that was
            // separately erasing this fetch-using fixture's Browser location (a NEW, narrower gap
            // than L5's own filing) is fixed too — confirmed GREEN. The Editor surface's own gap
            // (a cross-block check invisible to its per-block recovery reparse, L11_CROSS_BLOCK)
            // closed once fragment mode landed — confirmed GREEN too.
            page(
                "# Todos\n\n## A Todo has:\n    an id, which is Int.\n\n## Route GET /api/todo -> Todo:\n    \
                 Return a new Todo with id 1.\n\n## Component App\nState:\n    Let label be \"\".\nActions:\n    \
                 When the page loads:\n        Get \"/api/todo\" as Text, then loaded.\n    \
                 To loaded (r: Result of Text and HttpError):\n        Set label to \"x\".\nView:\n    \
                 <p>{label}</p>\n\n## Main\nMount App.\n"
                    .to_string(),
                "When the page loads",
                "this page fetches",
            ),
        ),
        r(
            Check("check_selectors_exist", "why"),
            page(
                component_acting("When \"ul > li\" is clicked:\n    Set n to 0.", "<ul>\n  <li>x</li>\n</ul>"),
                "When \"ul > li\"",
                "child combinator",
            ),
        ),
        r(
            Check("check_selectors_exist", "`{text}` names no element"),
            page(
                component_acting("When \"#nope\" is clicked:\n    Set n to 0.", "<div>\n  <p>x</p>\n</div>"),
                "When \"#nope\"",
                "names no element",
            ),
        ),
        r(
            Check("resolve_store_calls", "`{}` is not a"),
            page(
                component_acting("To leave:\n    Call Nope's login with \"x\".", "<div>\n  <p>x</p>\n</div>"),
                "Call Nope's",
                "is not a store",
            ),
        ),
        r(
            Check("resolve_store_calls", "`{}` is not an"),
            page(
                format!(
                    "## Store Session\nState:\n    Let user be \"\".\nActions:\n    To login (who: Text):\n        \
                     Set user to who.\n\n{}",
                    component_acting("To leave:\n    Call Session's logout.", "<div>\n  <p>x</p>\n</div>")
                ),
                "Call Session's logout",
                "is not an action",
            ),
        ),
        r(
            Check("mount_the_declared_page", "this file declares two"),
            page(
                "## Page /a\nView:\n    <p>a</p>\n\n## Page /b\nView:\n    <p>b</p>\n".to_string(),
                "## Page /b",
                "declares two pages",
            ),
        ),
        r(
            Check("check_mounted_components_need_no_props", "`Mount {0}` cannot run:"),
            page(
                "## Component Row (note: Text)\nView:\n    <p>{note}</p>\n\n## Main\nMount Row.\n".to_string(),
                "Mount Row",
                "a mount has no way to pass",
            ),
        ),
        r(
            Check("check_mounts_name_something", "`Mount {written}`: no component"),
            page(component("<p>x</p>").replace("Mount App.", "Mount Ap."), "Mount Ap.", "no component named"),
        ),
        // ── The name-resolution pass (`parser/lgx_resolve.rs`) ─────────────────────────────────
        r(
            Check("resolve_placement", "`<{tag}>` has no prop"),
            page(
                format!("## Component Card\nView:\n    <p>c</p>\n\n{}", component("<div>\n  <Card note={1}/>\n</div>")),
                "<Card note",
                "has no prop",
            ),
        ),
        r(
            Check("resolve_placement", "`<{tag}>` does not pass"),
            page(
                format!(
                    "## Component Card (note: Text)\nView:\n    <p>{{note}}</p>\n\n{}",
                    component("<div>\n  <Card/>\n</div>")
                ),
                "<Card/>",
                "does not pass",
            ),
        ),
        r(
            Check("resolve_placement", "message"),
            page(
                format!(
                    "## Component Card (note: Int)\nView:\n    <p>{{note}}</p>\n\n{}",
                    component("<div>\n  <Card note=\"x\"/>\n</div>")
                ),
                "<Card note",
                "and this value is a",
            ),
        ),
        r(
            Check("unknown_component", "{construct} names no component{guess}{stop}"),
            page(component("<div>\n  <Nowhere/>\n</div>"), "<Nowhere/>", "names no component"),
        ),
        r(
            Check("unresolved", "`{written}` is not defined"),
            page(component("<div>\n  <p>{drat}</p>\n</div>"), "{drat}", "is not defined in"),
        ),
        r(
            Check("resolve_handler", "`{name}` is a prop"),
            page(
                // A component may legally place itself; the mistake and the placement then sit in
                // the SAME view, so no other block's isolated reparse is missing a declaration
                // (`diag.md` "In progress": the editor's per-block recovery only replaces a
                // spurious cross-block finding when it lands on the mistake's own line).
                "## Component Child (note: Text)\nView:\n    <button on-click={note}>b</button>\n    \
                 <Child note=\"x\"/>\n"
                    .to_string(),
                "on-click={note}",
                "so an event has nothing to run",
            ),
        ),
        r(
            Check("resolve_handler", "`{written}` is not an"),
            page(component("<div>\n  <button on-click={nope}>b</button>\n</div>"), "on-click={nope}", "is not an action of"),
        ),
        r(
            Check("resolve_bind", "`bind={{{written}}}` would write `{written}`,"),
            page(
                "## Component Child (note: Text)\nView:\n    <input bind={note} aria-label=\"x\">\n    \
                 <Child note=\"x\"/>\n"
                    .to_string(),
                "bind={note}",
                "cannot change it",
            ),
        ),
        r(
            Check("resolve_bind", "`bind={{{written}}}` names no field"),
            page(component("<div>\n  <input bind={nope} aria-label=\"x\">\n</div>"), "bind={nope}", "names no field of"),
        ),
        r(
            Check("resolve_uses", "`Use {written}.` names no"),
            page(
                // `Use` sits at the END of its component (and the file) — nothing after it in the
                // same block for a per-block recovery reparse to stumble on (see the
                // `resolve_handler` row above).
                "## Page /x\nView:\n    <p>x</p>\nUse Nowhere.\n".to_string(),
                "Use Nowhere",
                "names no store",
            ),
        ),
        r(
            Check("resolve_journal_actions", "`{written}` is not an"),
            page(
                component("<p>x</p>")
                    .replace("## Main\nMount App.\n", "## Main\nMount App.\nShow the journal shows nope then go.\n"),
                "nope",
                "is not an action of any component",
            ),
        ),
        r(
            Check("resolve_store_writes", "`{}` reaches into the"),
            // The Editor surface's own gap (a cross-block check invisible to its per-block
            // recovery reparse, L11_CROSS_BLOCK) closed once fragment mode landed — confirmed
            // GREEN.
            page(
                format!(
                    "## Store Cart\nState:\n    Let count be 0.\nActions:\n    To add:\n        Set count to count + 1.\n\n{}",
                    component_acting("To bump:\n    Set Cart's count to 5.", "<p>x</p>")
                ),
                "Set Cart's count",
                "reaches into the store",
            ),
        ),
        r(
            Check("check_bare_action_typos_in", "`{written}` is not an"),
            // `Stmt::Call` carries no span of its own, so this refusal reports the ENCLOSING
            // action's span rather than the bad `Call`'s — `at` names the whole action to reflect
            // that honestly.
            page(
                component_acting("To bump:\n    Call goo.", "<p>x</p>"),
                "To bump:\n        Call goo.",
                "is not an action of",
            ),
        ),
        r(
            CopyLint("OrphanedCopy"),
            // `opens` is bound from `b's opens`, mutated by `Pop`, and never read or written back
            // — the lint's own canonical example (its module doc comment and
            // `parser::tests::pop_from_a_copied_field_and_never_writing_back_warns`). `at` is the
            // `Pop` statement — the finding's own span is its LAST MUTATION, not the `Let`.
            warning(
                "## A Box has:\n    an opens, which is Seq of Int.\n\n\
                 ## To useOpens (b: Box) -> Int:\n    Let opens be b's opens.\n    Pop from opens into at.\n    Return at.\n\n\
                 ## Main\nShow 1.\n"
                    .to_string(),
                "Pop from opens into at.",
                "is a copy of",
            ),
        ),
        r(
            Check("parse_trigger", "a dropped file binds"),
            // `span` is captured at `parse_trigger`'s own entry, before `When` is even consumed —
            // the whole trigger sentence, not just the mistyped param, so `at` names it entirely.
            page(
                component_acting(
                    "When a file is dropped on \"#zone\" (file: Int):\n    Show \"x\".",
                    "<div id=\"zone\">drop</div>",
                ),
                "When a file is dropped on \"#zone\" (file: Int):",
                "a dropped file binds",
            ),
        ),
        r(
            Check("trigger_error", "An `Actions:` sentence names"),
            // `parse_trigger` falls through to `trigger_error` when an `Actions:` sentence starts
            // with none of the fixed cues (`To`, `When`, `Every`, `After`) it recognizes.
            page(
                component_acting("Frobnicate the widget:\n    Set n to 0.", "<p>x</p>"),
                "Frobnicate the widget:",
                "An `Actions:` sentence names",
            ),
        ),
        r(
            Check("parse_param_groups", "ParseErrorKind::ExpectedKeyword"),
            // A trigger's parenthesized parameter list expects `name: Type`; a missing `:` is
            // reported at the token that stands where it belongs.
            page(
                component_acting("When an action fails (e Text):\n    Set n to 0.", "<p>x</p>"),
                "(e Text)",
                "needs the word ':' here",
            ),
        ),
        r(
            Check("parse_param_groups", "ParseErrorKind::ExpectedKeyword#2"),
            // A parameter group not closed by `)` after its last type is reported the same way, at
            // the token in that position instead.
            page(
                component_acting("When an action fails (e: Text extra):\n    Set n to 0.", "<p>x</p>"),
                "(e: Text extra)",
                "needs the word ')' here",
            ),
        ),
        r(
            Check("require_trigger_param_type", "{sentence} delivers `{want}`, never"),
            // `When an action fails (…):` delivers the failure's own message as `Text`; a parameter
            // annotated with a different single-named type is refused naming both.
            page(
                component_acting("When an action fails (e: Int):\n    Set n to 0.", "<p>x</p>"),
                "When an action fails (e: Int):",
                "delivers `Text`, never `Int`",
            ),
        ),
        r(
            Check("require_trigger_param_type", "{sentence} delivers `{want}`."),
            // A parameter annotated with a shape that names no single type (`Seq of Text`) cannot
            // be compared against `Text` by name, so the message states only what the trigger
            // delivers.
            page(
                component_acting("When an action fails (e: Seq of Text):\n    Set n to 0.", "<p>x</p>"),
                "When an action fails (e: Seq of Text):",
                "delivers `Text`.",
            ),
        ),
    ]
}
