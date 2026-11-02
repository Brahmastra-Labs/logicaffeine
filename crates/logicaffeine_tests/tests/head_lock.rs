//! ★ LOCK — a page owns its own `<head>`. ★
//!
//! Every built page's head came from `browser_host_html(stem)`, which hard-codes
//! `<title>{stem}</title>` — the FILE NAME. So a real site shipped `<title>app</title>`
//! on its front page and `<title>about</title>` on the next one, with no description,
//! no social card, no canonical URL. A link to a LOGOS site posted anywhere showed a
//! bare URL titled `app`.
//!
//! ★ THE SHAPE FOLLOWS THE FRAMEWORK'S OWN GRAMMAR. `View:` is real HTML and `Style:`
//! is real CSS, so `Head:` is real head HTML — parsed by the same LGX template parser,
//! so a hole works there like anywhere else.
//!
//! ★ AND IT MERGES, IT DOES NOT REPLACE. The built-in shell is a good default: charset,
//! viewport, a light/dark `theme-color`, a favicon. A page that declares a `<title>`
//! wants ITS title, not two of them; a page that declares nothing wants the shell it
//! has today, byte for byte. So a declared tag REPLACES the default with the same
//! identity (`title`, `meta[name]`, `meta[property]`, `link[rel]`) and anything else is
//! appended.

use logicaffeine_cli::prerender_page_html;

/// A page that says who it is — the ordinary head of a page worth sharing.
const ABOUT: &str = r#"## Page /about

Head:
    <title>About — Acme</title>
    <meta name="description" content="Who we are and why.">
    <meta property="og:title" content="About Acme">
    <meta property="og:image" content="/card.png">
    <link rel="canonical" href="https://acme.example/about">

View:
    <main>
      <h1>About</h1>
    </main>
"#;

/// A page with no `Head:` at all — the scaffold's shape.
const PLAIN: &str = r#"## Page /

View:
    <main>
      <h1>Hello</h1>
    </main>
"#;

fn head_of(html: &str) -> String {
    let start = html.find("<head>").expect("a document has a head");
    let end = html.find("</head>").expect("a document closes its head");
    html[start..end].to_string()
}

/// ★ THE LOAD-BEARING TEST — the title a person sees in a tab and a search result.
#[test]
fn a_page_titles_itself() {
    let html = prerender_page_html(ABOUT, "about");
    assert!(
        html.contains("<title>About — Acme</title>"),
        "the page's own title must reach the document:\n{html}"
    );
    assert!(
        !html.contains("<title>about</title>"),
        "and the file-name default must be GONE, not sitting beside it — two titles in one \
         document is a document with no title anybody can predict:\n{html}"
    );
}

/// A shared link is a card, and a card is these tags.
///
/// Asserted as a tag CARRYING both halves, not as an exact byte string: the document
/// renderer serializes attributes in a deterministic (sorted) order, which is a
/// load-bearing choice everywhere else in this suite — the op-log comparisons rest on
/// it. What has to be true here is that the pair reached the same element, which is
/// what a crawler reads; the order it reads them in is not a fact about this feature.
#[test]
fn a_page_carries_its_description_and_social_card() {
    let head = head_of(&prerender_page_html(ABOUT, "about"));
    /// Is there ONE tag in `head` carrying both of these attribute spellings?
    fn tag_with(head: &str, a: &str, b: &str) -> bool {
        head.split('<').any(|t| t.contains(a) && t.contains(b))
    }
    for (a, b) in [
        (r#"name="description""#, r#"content="Who we are and why.""#),
        (r#"property="og:title""#, r#"content="About Acme""#),
        (r#"property="og:image""#, r#"content="/card.png""#),
        (r#"rel="canonical""#, r#"href="https://acme.example/about""#),
    ] {
        assert!(
            tag_with(&head, a, b),
            "the head must carry one tag with {a} and {b} — both halves on the SAME element, \
             or the crawler reads a description belonging to nothing:\n{head}"
        );
    }
}

/// ★ THE DEFAULTS SURVIVE. A page that names a title has not opted out of being a
/// well-formed document — charset and viewport are not things anybody should have to
/// remember, and forgetting the viewport is what makes a site unreadable on a phone.
#[test]
fn declaring_a_head_keeps_the_defaults_it_did_not_override() {
    let head = head_of(&prerender_page_html(ABOUT, "about"));
    assert!(head.contains("charset=\"utf-8\""), "charset stays:\n{head}");
    assert!(
        head.contains("name=\"viewport\""),
        "the viewport meta stays:\n{head}"
    );
    assert!(head.contains("rel=\"icon\""), "the favicon stays:\n{head}");
    assert!(
        head.contains("theme-color"),
        "the light/dark theme-color stays:\n{head}"
    );
}

/// ★ AND A DECLARED TAG REPLACES THE DEFAULT OF THE SAME IDENTITY, rather than
/// appearing beside it. Two `theme-color` metas, or two viewports, is a document whose
/// behaviour depends on which one a browser happens to read last.
#[test]
fn a_declared_tag_replaces_the_default_with_the_same_identity() {
    let src = r#"## Page /

Head:
    <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1">

View:
    <main><h1>x</h1></main>
"#;
    let head = head_of(&prerender_page_html(src, "app"));
    assert_eq!(
        head.matches("name=\"viewport\"").count(),
        1,
        "exactly one viewport meta — the page's:\n{head}"
    );
    assert!(
        head.contains("maximum-scale=1"),
        "and it is the page's own:\n{head}"
    );
}

/// ★ A PAGE WITH NO `Head:` IS UNCHANGED. This whole feature must be additive: the
/// scaffold, and every page anybody has already written, keeps the document it has.
#[test]
fn a_page_with_no_head_section_is_untouched() {
    let html = prerender_page_html(PLAIN, "app");
    assert!(
        html.contains("<title>app</title>"),
        "with no `Head:`, the shell's file-name title is still what ships:\n{html}"
    );
    assert!(
        html.contains("Hello"),
        "and the body is pre-rendered as before:\n{html}"
    );
}

/// A hole works in the head like anywhere else — a title that reads state is the
/// commonest reason to want one.
#[test]
fn a_head_may_interpolate_state() {
    let src = r#"## Page /

State:
    Let who be "Ada".

Head:
    <title>Hello {who}</title>

View:
    <main><h1>hi</h1></main>
"#;
    let html = prerender_page_html(src, "app");
    assert!(
        html.contains("<title>Hello Ada</title>"),
        "the hole renders:\n{html}"
    );
}

/// ★ THE HEAD IS NOT A PLACE FOR A `<div>`. A body element in the head is silently
/// relocated by every browser, which moves it OUT of the head and into the body where
/// it renders — so a mistake there produces a page that looks wrong for a reason
/// nothing explains. Refuse it by name instead.
#[test]
fn a_body_element_in_the_head_is_refused_by_name() {
    let src = r#"## Page /

Head:
    <div>oops</div>

View:
    <main><h1>x</h1></main>
"#;
    // A PARSE error, so it is reported at the mistake rather than at some later tier.
    let err = logicaffeine_compile::compile_to_rust(src)
        .err()
        .map(|e| format!("{e:?}"));
    let msg = err.unwrap_or_else(|| {
        panic!(
            "a <div> in a `Head:` must be refused, not compiled: {}",
            prerender_page_html(src, "app")
        )
    });
    assert!(
        msg.contains("head") || msg.contains("Head"),
        "the refusal must name the head, so the reader knows which section is wrong: {msg}"
    );
}

// -------------------------------------------------------------------------------------------
// ★ THE HEAD SURVIVES A BODY THAT WILL NOT PRE-RENDER.
// -------------------------------------------------------------------------------------------

/// A page whose BODY will not render — `<Nav/>` is not declared anywhere, which is a typo, the
/// most ordinary way a page stops drawing — but whose head is static markup that has nothing to do
/// with it.
const BROKEN_BODY: &str = r#"## Page /post

Head:
    <title>A Post — Acme</title>
    <meta name="description" content="Something worth finding.">

State:
    Let xs be ["only"].

View:
    <main>
      <h1>A Post</h1>
      <p>{item 5 of xs}</p>
    </main>
"#;

#[test]
fn a_page_that_cannot_pre_render_still_titles_itself() {
    // ★ THE HEAD WAS THROWN AWAY WITH THE BODY. `prerendered_html` matched
    // `let Ok(markup) = render_page_markup(source) else { … browser_host_html_with_head(stem, None,
    // …) }` — passing `None` for the head — so a page whose body would not render shipped
    // `<title>post</title>`, the FILE NAME, with no description and no social card.
    //
    // That is the wrong half to lose, and it is the half that does not depend on the failure: a
    // `<head>` is static markup, the body is what used the browser-only construct. Pre-rendering is
    // "an improvement, never a new way to fail" — dropping a title the author wrote IS a new way to
    // fail, and the one a crawler and a shared link notice first.
    let html = prerender_page_html(BROKEN_BODY, "post");
    assert!(
        html.contains("<title>A Post — Acme</title>"),
        "a page whose BODY cannot pre-render must still carry the head it declared — losing the \
         title is losing the one thing a crawler and a shared link read:\n{html}"
    );
    assert!(
        html.contains("Something worth finding."),
        "and the rest of its head with it:\n{html}"
    );
    assert!(
        !html.contains("<title>post</title>"),
        "and must NOT fall back to the file name:\n{html}"
    );
}

#[test]
fn the_shell_still_ships_when_the_body_cannot_be_drawn() {
    // ★ THE RULE THIS MUST NOT BREAK: pre-rendering is an improvement, never a new way to fail.
    // Recovering the head must not turn an undrawable body into a build error, or into a document
    // missing the script that makes the page work in a browser.
    let html = prerender_page_html(BROKEN_BODY, "post");
    assert!(
        html.contains("<script"),
        "the module must still load:\n{html}"
    );
    assert!(
        html.contains("<body"),
        "the document must still be a document:\n{html}"
    );
}

#[test]
fn a_page_with_no_head_and_an_undrawable_body_is_unchanged() {
    // The floor: with nothing declared, the shell it has today, byte for byte.
    const PLAIN_BROKEN: &str = "## Page /x\n\nView:\n    <main><Nav/></main>\n";
    let html = prerender_page_html(PLAIN_BROKEN, "x");
    assert!(
        html.contains("<title>x</title>"),
        "the default title stands:\n{html}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ A PAGE OWNS ITS LANGUAGE TOO, AND `en` WAS WELDED IN.
//
// `<html lang="en">` was a string literal in the document shell, so every page
// LOGOS has ever built claimed to be in English. That is not cosmetic: `lang`
// is what a screen reader picks a voice from, what a browser offers to
// translate on, and what hyphenation and quotation rules key off. A French site
// built with this framework shipped French prose annotated as English, and
// nothing anywhere let the author say otherwise.
//
// ★ IT IS DECLARED IN `Head:`, WHICH IS WHERE A PAGE ALREADY SAYS WHO IT IS.
// No new syntax: `Head:` is real head HTML, and `<html lang="…"></html>` is the
// idiom every framework with a document shell settled on for exactly this. The
// tag is LIFTED onto the document element rather than rendered — an `<html>`
// inside `<head>` would be malformed, and the author's intent is unambiguous.
//
// ★ IT IS WRITTEN CLOSED, and that is the template parser's rule rather than a
// preference: `html` is not a void element, so an unclosed `<html lang="fr">`
// opens an element that swallows every tag after it. Writing the close is one
// character of honesty about what the markup means.
//
// ★ AND THE OTHER THREE `lang="en"` SITES STAY. The audit counted four. Three
// are the TOOL's own chrome — the build-error overlay, the preview shell, and
// `largo serve`'s dev panel — which are written in English by us and are not
// anybody's page. Changing those would be following a grep rather than the
// argument.
// ─────────────────────────────────────────────────────────────────────────────

/// A page that says what language it is in.
const FRENCH: &str = r#"## Page /apropos

Head:
    <html lang="fr"></html>
    <title>À propos</title>

View:
    <main>
      <h1>À propos</h1>
    </main>
"#;

#[test]
fn a_page_may_declare_its_own_language() {
    let html = prerender_page_html(FRENCH, "apropos");
    assert!(
        html.contains("<html lang=\"fr\">"),
        "the page declared `<html lang=\"fr\">` and the document must carry it — `lang` is what a \
         screen reader picks a voice from:\n{html}"
    );
    assert!(
        !html.contains("<html lang=\"en\">"),
        "the welded-in default must be gone, not merely joined:\n{html}"
    );
}

#[test]
fn the_language_tag_is_lifted_and_not_rendered_into_the_head() {
    // An `<html>` element inside `<head>` is malformed markup. The declaration is a
    // statement ABOUT the document, so it moves to the document element.
    let html = prerender_page_html(FRENCH, "apropos");
    let head = html
        .split("<head>")
        .nth(1)
        .and_then(|h| h.split("</head>").next())
        .unwrap_or("");
    assert!(
        !head.contains("<html"),
        "the `<html lang>` declaration was rendered inside <head>, which is malformed:\n{head}"
    );
    // …and the rest of the head still merged as it always did.
    assert!(
        head.contains("À propos"),
        "the declared title must survive the lift:\n{head}"
    );
}

#[test]
fn a_page_that_says_nothing_is_still_english() {
    // ★ THE CONTROL. Every existing page declares no language and must keep the exact
    // document it has today — this is a new capability, not a change of default.
    let html = prerender_page_html(PLAIN, "index");
    assert!(
        html.contains("<html lang=\"en\">"),
        "a page with no declaration keeps the default:\n{html}"
    );
}

// ── a layout's shared head ────────────────────────────────────────────────────────────────────────

/// A layout carrying the site's shared head, and a page that extends it and names its own title.
const SITE_WITH_HEAD: &str = r#"## Component Site

Head:
    <title>Acme</title>
    <meta name="description" content="Acme makes things.">
    <meta property="og:site_name" content="Acme">

View:
    <header>Acme</header>
    <main>{block content}</main>

## Page /about

Head:
    <title>About — Acme</title>

View:
    {extends Site}
    {block content:}<h1>About</h1>{end}
"#;

/// The same layout, and a page that declares no head of its own.
const SITE_PAGE_WITHOUT_HEAD: &str = r#"## Component Site

Head:
    <title>Acme</title>
    <meta name="description" content="Acme makes things.">

View:
    <main>{block content}</main>

## Page /

View:
    {extends Site}
    {block content:}<h1>Home</h1>{end}
"#;

/// The document head the lowered VM composes when the page at `page_path` mounts into `body`.
fn lowered_head(src: &str, page_path: &str) -> String {
    use logicaffeine_compile::interpreter::RuntimeValue;
    use logicaffeine_compile::semantics::dom;
    use logicaffeine_compile::ui_bridge::with_parsed_program;
    use logicaffeine_compile::vm::session::Session;
    use logicaffeine_compile::vm::{Compiler, Vm};
    use logicaffeine_language::parser::lgx_lower::mount_fn_name;
    dom::reset();
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) =
            parsed.unwrap_or_else(|e| panic!("the page parses:\n{src}\n=> {e}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("the page compiles:\n{src}\n=> {e}"));
        let mut session = Session::new(Vm::new(&program), interner);
        session
            .run_main()
            .unwrap_or_else(|e| panic!("main runs: {e}"));
        let page = format!("__page_{}", page_path.replace(['/', ':'], "_"));
        session
            .invoke(&mount_fn_name(&page), vec![RuntimeValue::Int(dom::body())])
            .unwrap_or_else(|e| panic!("the page mounts: {e}"));
    });
    dom::document_head().unwrap_or_default()
}

fn assert_merged_under_the_page(head: &str, tier: &str) {
    assert_eq!(
        head.matches("<title>").count(),
        1,
        "{tier}: one title in the document:\n{head}"
    );
    assert!(
        head.contains("<title>About — Acme</title>"),
        "{tier}: the page's own title wins:\n{head}"
    );
    assert!(
        head.contains("Acme makes things."),
        "{tier}: the layout's description stands:\n{head}"
    );
    assert!(
        head.contains("og:site_name"),
        "{tier}: the layout's social tag stands:\n{head}"
    );
}

#[test]
fn a_layouts_head_merges_under_the_pages_head_on_the_lowered_vm() {
    assert_merged_under_the_page(&lowered_head(SITE_WITH_HEAD, "/about"), "lowered VM");
}

#[test]
fn a_layouts_head_merges_under_the_pages_head_in_the_prerendered_document() {
    assert_merged_under_the_page(
        &head_of(&prerender_page_html(SITE_WITH_HEAD, "about")),
        "prerender",
    );
}

#[test]
fn a_page_with_no_head_takes_its_layouts_head_on_every_tier() {
    let lowered = lowered_head(SITE_PAGE_WITHOUT_HEAD, "/");
    let prerendered = head_of(&prerender_page_html(SITE_PAGE_WITHOUT_HEAD, "app"));
    for (tier, head) in [("lowered VM", lowered), ("prerender", prerendered)] {
        assert!(
            head.contains("<title>Acme</title>"),
            "{tier}: the layout's title:\n{head}"
        );
        assert!(
            !head.contains("<title>app</title>"),
            "{tier}: not the file-name default:\n{head}"
        );
        assert!(
            head.contains("Acme makes things."),
            "{tier}: the layout's description:\n{head}"
        );
    }
}

/// ★ A PLACED COMPONENT DOES NOT OWN THE DOCUMENT. A `Head:` belongs to what owns it — a page, or
/// the layout a page extends — and two placed components each naming a title would make the
/// document's title depend on render order.
#[test]
fn a_placed_component_carrying_a_head_is_refused_at_the_placement() {
    let src = r#"## Component Card

Head:
    <title>Card</title>

View:
    <section>card</section>

## Page /

View:
    <main><Card/></main>
"#;
    let err = logicaffeine_compile::compile::compile_to_wasm(src)
        .expect_err("a placed component's head is refused");
    let why = format!("{err:?}");
    assert!(
        why.contains("`<Card/>`") && why.contains("`Head:`"),
        "the refusal names the placement and the section: {why}"
    );
    assert!(
        why.contains("layout"),
        "and teaches where a shared head belongs: {why}"
    );
}

/// ★ THE DECLARED HEAD IS MERGED ONCE, where it is recorded. A page's head arrives as one string —
/// its layout's `Head:` and then its own — and the document merges it tag by tag, a later tag of an
/// identity replacing an earlier one. What a server composes the page's document from
/// (`dom::head_html`) is that same merge, each surviving tag exactly as the author wrote it — or a
/// prerendered page carries its layout's title AND its own.
#[test]
fn the_declared_head_is_the_merged_head_each_tag_as_written() {
    use logicaffeine_compile::semantics::dom;
    dom::reset();
    dom::set_head_html(
        "<title>Acme</title><meta name=\"description\" content=\"Acme makes things.\"><title>About — Acme</title>",
    );
    assert_eq!(
        dom::head_html().as_deref(),
        Some(
            "<meta name=\"description\" content=\"Acme makes things.\"><title>About — Acme</title>"
        ),
        "one title, the later one, joining at the end; the description as written"
    );
}
