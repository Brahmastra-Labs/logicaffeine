//! ★ LOCK — a hole is TEXT, whatever a stranger puts in it. ★
//!
//! Every hole in a LOGOS page can carry data the author did not write: a path segment, a database
//! row, a form field. Unescaped, one visitor's input runs script in every other visitor's browser.
//!
//! THE ASSERTION IS STRUCTURAL, not a substring search. A payload is rendered, and the SHAPE of
//! the resulting document is compared against the same page rendered with a harmless string: same
//! element count, same attribute count. A hole that stayed text cannot change either. String
//! checks like "the output has no `<script`" miss every payload whose spelling you did not think
//! of, and pass happily on `<SCRIPT`, `<img/onerror>` or a broken-out attribute — which is exactly
//! how an escaping bug survives a test suite.
//!
//! Both POSITIONS are covered, because they fail differently: in element content a payload adds
//! markup; in an ATTRIBUTE VALUE a bare quote ends the attribute and everything after it becomes
//! markup nobody wrote. And both TIERS a stranger's data reaches: the browser page and the
//! server-rendered document.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

/// The corpus. Chosen so each entry breaks a DIFFERENT naive escape: a missing `<`, a missing
/// quote, an attribute broken out of, a tag that needs no closing, a comment that ends early,
/// text that is already escaped, and a case-varied spelling.
const PAYLOADS: &[(&str, &str)] = &[
    ("classic script tag", "<script>alert(1)</script>"),
    ("no closing tag needed", "<img src=x onerror=alert(1)>"),
    ("breaks out of an attribute", "\"><script>alert(1)</script>"),
    (
        "breaks out with a single quote",
        "'><script>alert(1)</script>",
    ),
    ("svg autoload", "<svg/onload=alert(1)>"),
    ("case varied", "<ScRiPt>alert(1)</ScRiPt>"),
    (
        "closes a textarea first",
        "</textarea><script>alert(1)</script>",
    ),
    ("hides in a comment", "<!--<script>alert(1)</script>-->"),
    (
        "already escaped, must not be un-escaped",
        "&lt;script&gt;alert(1)&lt;/script&gt;",
    ),
    ("bare ampersand", "a & b"),
    ("a quote alone", "say \"hi\""),
    ("a backslash", "c:\\path\\to"),
    ("angle brackets only", "<>"),
    ("javascript url", "javascript:alert(1)"),
];

/// A string with no markup meaning at all — the shape oracle.
const BENIGN: &str = "harmless";

/// Render a page whose state field holds `value`, and report `(elements, attributes)`.
fn shape(template: &str, value: &str, at: Position) -> usize {
    // The value is spelled as BYTES so no payload can escape the LOGOS source itself and change
    // what program is being tested — a test whose fixture is itself injectable proves nothing.
    let bytes: Vec<String> = value.bytes().map(|b| b.to_string()).collect();
    let src = format!(
        "## Component App\nState:\n    Let v be text_from_bytes([{}]).\nView:\n    {template}\n\n\
         ## Main\nMount App.\n",
        bytes.join(", ")
    );
    dom::reset();
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "the page must render:\n{src}");
    structure_at(&dom::render(), at)
}

/// Which characters are STRUCTURAL depends on WHERE the hole is, and getting that wrong is how a
/// corpus like this accuses a renderer that is doing its job.
///
/// - In element TEXT, `<` and `>` open and close tags. A quote is ordinary text there — HTML
///   requires no escaping of it — so counting quotes reported a difference for
///   `"><script>…` when the renderer had correctly left a harmless quote alone.
/// - In an ATTRIBUTE VALUE, a quote ENDS the attribute and everything after it becomes markup
///   nobody wrote, so `"` is structural there and must be counted.
/// - `=` is structural in neither: it appears inside escaped text (`&lt;img src=x&gt;` has two).
#[derive(Clone, Copy)]
enum Position {
    Text,
    Attribute,
}

fn structure_at(html: &str, at: Position) -> usize {
    let base = html.matches('<').count() + html.matches('>').count();
    match at {
        Position::Text => base,
        Position::Attribute => base + html.matches('"').count(),
    }
}

fn structure(html: &str) -> usize {
    structure_at(html, Position::Attribute)
}

/// The same, for the SERVER-rendered document.
fn ssr_shape(template: &str, value: &str, at: Position) -> usize {
    let bytes: Vec<String> = value.bytes().map(|b| b.to_string()).collect();
    let src = format!(
        "## Component App\nState:\n    Let v be text_from_bytes([{}]).\nView:\n    {template}\n\n\
         ## Main\nShow html of App.\n",
        bytes.join(", ")
    );
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "the page must server-render:\n{src}");
    structure_at(&out.output, at)
}

#[test]
fn no_payload_changes_the_shape_of_a_page_through_a_text_hole() {
    let template = "<div class=\"card\"><p>{v}</p></div>";
    let want = shape(template, BENIGN, Position::Text);
    for (label, payload) in PAYLOADS {
        let got = shape(template, payload, Position::Text);
        assert_eq!(
            got, want,
            "`{label}` changed the document's shape through a TEXT hole — it became markup. \
             Payload: {payload:?}"
        );
    }
}

#[test]
fn no_payload_changes_the_shape_of_a_page_through_an_attribute_hole() {
    // The position a bare quote breaks out of. It fails differently from element content, so it
    // gets its own pass over the whole corpus.
    let template = "<div class=\"card\"><p title={v}>x</p></div>";
    let want = shape(template, BENIGN, Position::Attribute);
    for (label, payload) in PAYLOADS {
        let got = shape(template, payload, Position::Attribute);
        assert_eq!(
            got, want,
            "`{label}` changed the document's shape through an ATTRIBUTE hole — a quote ended \
             the attribute and the rest became markup. Payload: {payload:?}"
        );
    }
}

#[test]
fn no_payload_changes_the_shape_of_a_server_rendered_text_hole() {
    // The tier whose output reaches people who never loaded the app.
    let template = "<div class=\"card\"><p>{v}</p></div>";
    let want = ssr_shape(template, BENIGN, Position::Text);
    for (label, payload) in PAYLOADS {
        let got = ssr_shape(template, payload, Position::Text);
        assert_eq!(
            got, want,
            "`{label}` became markup in SERVER-rendered content: {payload:?}"
        );
    }
}

#[test]
fn no_payload_changes_the_shape_of_a_server_rendered_attribute_hole() {
    let template = "<div class=\"card\"><p title={v}>x</p></div>";
    let want = ssr_shape(template, BENIGN, Position::Attribute);
    for (label, payload) in PAYLOADS {
        let got = ssr_shape(template, payload, Position::Attribute);
        assert_eq!(
            got, want,
            "`{label}` broke out of a SERVER-rendered attribute: {payload:?}"
        );
    }
}

#[test]
fn the_shape_oracle_actually_notices_markup() {
    // ★ THE TEST'S OWN TEETH. If the shape measure could not tell markup from text, every
    // assertion above would pass against a renderer that escaped NOTHING. `{raw}` is the one
    // door to real markup, so it is the control: the same payload through it MUST change the
    // shape. Without this control the corpus is decoration.
    let raw = "## Component App\nState:\n    Let v be \"<b>x</b>\".\nView:\n    \
        <div class=\"card\"><p>{raw v}</p></div>\n\n## Main\nMount App.\n";
    dom::reset();
    assert_eq!(tw_outcome(raw).error, None);
    let with_markup = dom::render();
    let benign = shape(
        "<div class=\"card\"><p>{v}</p></div>",
        "<b>x</b>",
        Position::Attribute,
    );
    assert_ne!(
        structure(&with_markup),
        benign,
        "`{{raw}}` must produce MORE markup than an escaped hole — otherwise this file's shape \
         measure is blind and every test in it is vacuous"
    );
}

#[test]
fn an_already_escaped_payload_is_not_double_unescaped() {
    // `&lt;script&gt;` must survive as VISIBLE TEXT reading `<script>`, not be turned back into a
    // tag. A renderer that unescaped on the way in would hand an attacker a two-step bypass.
    let bytes: Vec<String> = "&lt;script&gt;".bytes().map(|b| b.to_string()).collect();
    let src = format!(
        "## Component App\nState:\n    Let v be text_from_bytes([{}]).\nView:\n    <p>{{v}}</p>\n\n\
         ## Main\nMount App.\n",
        bytes.join(", ")
    );
    dom::reset();
    assert_eq!(tw_outcome(&src).error, None);
    let html = dom::render();
    assert!(
        !html.contains("<script"),
        "it must not become a tag: {html}"
    );
    assert!(
        html.contains("&amp;lt;"),
        "the ampersand itself is escaped: {html}"
    );
}

// ---------------------------------------------------------------------------------------------
// ★ URL POSITION — the one the shape oracle above is BLIND to. ★
//
// The corpus has carried `("javascript url", "javascript:alert(1)")` since it was written, and it
// has never once been able to fail on it: `structure_at` counts `<`, `>` and `"`, and that payload
// contains none of them. So the entry passed in both positions on both tiers while doing nothing —
// and the corpus is cited as the evidence that holes are safe.
//
// ★★ A CORPUS THAT CONTAINS THE ATTACK AND CANNOT SEE IT IS WORSE THAN ONE THAT OMITS IT, because
// it is read as coverage. This is the same silent specification as a fuzzer whose alphabet is
// ASCII and does not say so (`lgx_non_ascii_lock`), and as an `every_op()` that was missing an op:
// the gap was in what was never enumerated, not in what was decided wrongly.
//
// A URL hole needs a DIFFERENT oracle, because the danger is not shape. `javascript:alert(1)` in
// an `href` adds no element and no attribute; it is dangerous because of what the BROWSER does
// when the link is followed. So the property here is about the scheme that reaches the document,
// not about the document's structure.

/// Schemes a browser will EXECUTE, rather than fetch. `data:` is here because a `data:text/html`
/// document runs script in the origin that opened it.
const EXECUTABLE_SCHEMES: &[&str] = &["javascript:", "data:", "vbscript:"];

/// Every URL-valued attribute a view can write. Each one is a navigation the browser performs on
/// the author's behalf, so each one is a place a stranger's string becomes an action.
const URL_ATTRIBUTES: &[(&str, &str)] = &[
    ("a", "href"),
    ("img", "src"),
    ("form", "action"),
    ("iframe", "src"),
];

/// Payloads that carry an executable scheme past a shape check. Every one of these renders with
/// the same element and attribute count as a benign URL.
const URL_PAYLOADS: &[(&str, &str)] = &[
    ("plain javascript url", "javascript:alert(1)"),
    ("cased scheme", "JaVaScRiPt:alert(1)"),
    ("whitespace before the scheme", "  javascript:alert(1)"),
    ("tab inside the scheme", "java\tscript:alert(1)"),
    ("newline inside the scheme", "java\nscript:alert(1)"),
    (
        "data url running html",
        "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==",
    ),
    ("vbscript", "vbscript:msgbox(1)"),
];

/// A URL that is perfectly ordinary and must keep working — the other half of the property. A
/// guard that blocked these would be worse than the hole it closes.
const SAFE_URLS: &[&str] = &[
    "/about",
    "https://example.com/x?a=b#c",
    "mailto:hi@example.com",
    "tel:+15551234",
    "#section",
    "?page=2",
    "//example.com/protocol-relative",
    "images/logo.png",
];

/// The element as a view writes it. A VOID element takes no children and no end tag — `</img>` is
/// not markup HTML has, and the parser is right to refuse it.
///
/// ★ MY FIRST DRAFT OF THIS FIXTURE EMITTED ONE, and the failure read "the page must render" rather
/// than anything about escaping. A fixture that cannot parse proves nothing about the guard it was
/// written to test, and it fails in a way that looks like the guard is missing.
fn url_markup(tag: &str, attr: &str) -> String {
    if logicaffeine_language::lgx::html::is_void(tag) {
        format!("<{tag} {attr}={{v}}>")
    } else {
        format!("<{tag} {attr}={{v}}>x</{tag}>")
    }
}

/// Render `<tag attr={v}>` and hand back the document.
fn url_doc(tag: &str, attr: &str, value: &str) -> String {
    let bytes: Vec<String> = value.bytes().map(|b| b.to_string()).collect();
    let src = format!(
        "## Component App\nState:\n    Let v be text_from_bytes([{}]).\nView:\n    {}\n\n\
         ## Main\nMount App.\n",
        bytes.join(", "),
        url_markup(tag, attr)
    );
    dom::reset();
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "the page must render:\n{src}");
    dom::render()
}

/// The same, server-rendered — a stranger's data reaches both documents.
fn url_doc_ssr(tag: &str, attr: &str, value: &str) -> String {
    let bytes: Vec<String> = value.bytes().map(|b| b.to_string()).collect();
    let src = format!(
        "## Component App\nState:\n    Let v be text_from_bytes([{}]).\nView:\n    {}\n\n\
         ## Main\nShow html of App.\n",
        bytes.join(", "),
        url_markup(tag, attr)
    );
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "the page must server-render:\n{src}");
    out.output
}

/// Whether `doc` still carries a scheme a browser would execute, ignoring the case and the
/// whitespace a browser itself strips before deciding what a URL means.
fn carries_executable_scheme(doc: &str) -> Option<&'static str> {
    let flat: String = doc
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    EXECUTABLE_SCHEMES
        .iter()
        .copied()
        .find(|s| flat.contains(s))
}

#[test]
fn a_dangerous_scheme_never_reaches_a_url_attribute() {
    for (tag, attr) in URL_ATTRIBUTES {
        for (label, payload) in URL_PAYLOADS {
            let doc = url_doc(tag, attr, payload);
            assert!(
                carries_executable_scheme(&doc).is_none(),
                "`{label}` put an executable scheme into <{tag} {attr}>. A visitor who follows \
                 this link runs the payload in the page's own origin, and the shape oracle above \
                 cannot see it because the document's structure never changed.\nPayload: \
                 {payload:?}\nDocument: {doc}"
            );
        }
    }
}

#[test]
fn a_dangerous_scheme_never_reaches_a_server_rendered_url_attribute() {
    // The server-rendered document is what a crawler, a slow connection and a reader with no
    // script receive — and a link in it is followed by exactly the same browser.
    for (tag, attr) in URL_ATTRIBUTES {
        for (label, payload) in URL_PAYLOADS {
            let doc = url_doc_ssr(tag, attr, payload);
            assert!(
                carries_executable_scheme(&doc).is_none(),
                "`{label}` put an executable scheme into a server-rendered <{tag} {attr}>.\n\
                 Payload: {payload:?}\nDocument: {doc}"
            );
        }
    }
}

#[test]
fn an_ordinary_url_still_arrives_intact() {
    // The half that keeps the guard honest. A scheme check that also broke `/about`, `mailto:` or
    // a fragment would be traded for a worse bug, and "nothing dangerous got through" is trivially
    // satisfiable by rendering nothing at all.
    for url in SAFE_URLS {
        let doc = url_doc("a", "href", url);
        assert!(
            doc.contains(url),
            "an ordinary URL must survive untouched, or the guard costs more than it saves.\n\
             URL: {url:?}\nDocument: {doc}"
        );
    }
}

#[test]
fn the_scheme_oracle_actually_notices_one() {
    // The anti-vacuity guard, the same role `the_shape_oracle_actually_notices_markup` plays for
    // the tests above: prove the detector can fire, so a green run means "nothing dangerous got
    // through" rather than "the check was blind".
    assert!(carries_executable_scheme("<a href=\"javascript:alert(1)\">x</a>").is_some());
    assert!(carries_executable_scheme("<a href=\"JAVASCRIPT:alert(1)\">x</a>").is_some());
    assert!(carries_executable_scheme("<a href=\"java script:alert(1)\">x</a>").is_some());
    assert!(carries_executable_scheme("<a href=\"/about\">x</a>").is_none());
    assert!(carries_executable_scheme("<a href=\"mailto:hi@example.com\">x</a>").is_none());
}

// ---------------------------------------------------------------------------------------------
// ★ INLINE EVENT HANDLERS — the sink escaping cannot help with. ★
//
// `<button onclick={v}>` is an ordinary `Attr::Plain` today. The SSR path emits `onclick="…"` with
// `& < > "` escaped, and the browser path calls `setAttribute("onclick", …)` — which creates a live
// inline handler. **HTML-escaping does nothing in a JavaScript context**: the value is not markup
// being neutralised, it is source being executed. Every payload in the corpus above survives it.
//
// ★ AND IT IS A FOOTGUN BEFORE IT IS A HOLE. The framework's own spelling is `on-click={action}`
// with a hyphen. An author who types `onclick=` gets no diagnostic and no action — they get a raw
// HTML handler carrying the *stringified value* of their expression. The page looks wired up and
// answers nothing, which is the failure shape this campaign has now found in `bind`, in the anchor
// with no default action, and in the checkbox that rendered right and could not be ticked.
//
// The fix is the one `<script>` already gets: refuse it, by name, at compile time. `lgx/html.rs`
// rejects `<script>` because behaviour belongs in `Actions:`; an `on*` attribute is that same
// sentence written as an attribute.
//
// NOTE FOR THE IMPLEMENTER: the rule should be `on` + an ASCII letter, not a list of event names —
// a list is the thing that goes stale, and the whole point of this campaign is that the durable fix
// enumerates the class rather than its current members. That does also refuse a hypothetical custom
// attribute like `once`; refusing is the safe direction, and no standard HTML attribute begins with
// `on` without being a handler.

/// The handlers a page is most likely to carry, and the ones an attacker reaches for.
const INLINE_HANDLERS: &[&str] = &["onclick", "onerror", "onload", "onmouseover", "onfocus"];

#[test]
fn an_inline_event_handler_attribute_is_refused() {
    for name in INLINE_HANDLERS {
        let src = format!(
            "## Component App\nView:\n    <button {name}=\"alert(1)\">go</button>\n\n\
             ## Main\nMount App.\n"
        );
        let err = tw_outcome(&src).error.unwrap_or_else(|| {
            panic!(
                "`{name}` is a JavaScript sink: escaping is meaningless there, so it must be \
                 refused the way `<script>` is.\n{src}"
            )
        });
        assert!(
            err.contains(name) || err.to_lowercase().contains("on-"),
            "the refusal must name the attribute or point at the `on-` form: {err}"
        );
    }
}

#[test]
fn an_inline_handler_holding_a_hole_is_refused_too() {
    // The dangerous shape: a stranger's text landing in a JavaScript context. Escaping runs and
    // accomplishes nothing, so the shape oracle above would call this page unchanged.
    let src = "## Component App\nState:\n    Let v be \"x\".\nView:\n    \
               <button onclick={v}>go</button>\n\n## Main\nMount App.\n";
    assert!(
        tw_outcome(src).error.is_some(),
        "a hole in an inline handler is a stranger's data in a script position:\n{src}"
    );
}

#[test]
fn the_framework_s_own_handler_form_still_works() {
    // The other half, and the reason the refusal has to name `on-`: the fix must send the author to
    // the spelling that works, not merely forbid the one that does not.
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        \
               Set n to n + 1.\nView:\n    <button on-click={bump}>{n}</button>\n\n\
               ## Main\nMount App.\n";
    assert_eq!(
        tw_outcome(src).error,
        None,
        "`on-click={{action}}` is the supported form:\n{src}"
    );
}

#[test]
fn an_ordinary_attribute_beginning_with_on_is_not_collateral() {
    // The guard keys on `on` + a letter, so it must not swallow an attribute that merely starts
    // with those two characters as part of a longer word an author legitimately wrote.
    let src = "## Component App\nView:\n    <p data-online=\"yes\">x</p>\n\n## Main\nMount App.\n";
    assert_eq!(
        tw_outcome(src).error,
        None,
        "`data-online` is not an event handler:\n{src}"
    );
}
