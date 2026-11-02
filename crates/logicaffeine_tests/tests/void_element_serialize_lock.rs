//! ★ LOCK — a VOID element is serialized without a closing tag. ★
//!
//! `<input>`, `<br>`, `<img>`, `<hr>`, `<meta>` and their siblings have no content and no end tag;
//! HTML's own parser treats `</input>` as a stray tag and DROPS it. The DOM recorder's serializer
//! closed every element it wrote, so a mounted page came out as `<input …></input>`.
//!
//! ★ WHERE IT REACHES PEOPLE: `largo build` pre-renders every page by MOUNTING it and serializing
//! the recorded document (T6.8), so this markup is exactly what a crawler, a slow connection and a
//! reader with scripts off receive. The server-side `-> View` walk gets it right — it consults
//! `lgx::html::is_void` — so the two renderers of the same page disagreed about the same element,
//! which is the drift a single source of truth exists to prevent.
//!
//! ★ WHY IT IS NOT COSMETIC. A browser recovering from a stray `</input>` is a browser guessing,
//! and what it guesses is not always what follows. The `<br></br>` case is the notorious one: some
//! parsers treat the stray close as a SECOND `<br>`, so a page that renders one line break served
//! two. Nothing warns; the page is simply a little wrong.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

/// The markup a MOUNTED page serializes to — the same serializer `largo build` pre-renders with.
fn html_of(view: &str) -> String {
    dom::reset();
    let src = format!("## Component App\n\nView:\n{view}\n\n## Main\nMount App.\n");
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "{src}");
    dom::render()
}

#[test]
fn an_input_has_no_closing_tag() {
    // ★ THE ONE EVERY FORM HAS.
    let h = html_of("    <form><input id=\"draft\" type=\"text\"></form>");
    assert!(h.contains("<input"), "the input must render: {h}");
    assert!(
        !h.contains("</input>"),
        "a void element must not be closed: {h}"
    );
}

#[test]
fn every_void_element_is_left_open() {
    // Swept rather than sampled: the next one added is the next one forgotten, and each of these
    // is a tag an ordinary page writes.
    for tag in [
        "br", "hr", "img", "input", "meta", "link", "area", "col", "embed", "source", "track",
        "wbr",
    ] {
        let h = html_of(&format!("    <div><{tag}></div>"));
        assert!(h.contains(&format!("<{tag}")), "<{tag}> must render: {h}");
        assert!(
            !h.contains(&format!("</{tag}>")),
            "<{tag}> must not be closed: {h}"
        );
    }
}

#[test]
fn an_ordinary_element_is_still_closed() {
    // The other side of the rule — a fix that stopped closing everything would produce markup no
    // browser could nest correctly, and every assertion above would still pass.
    let h = html_of("    <div><p>hi</p><span>there</span></div>");
    assert!(
        h.contains("</p>") && h.contains("</span>") && h.contains("</div>"),
        "content elements close: {h}"
    );
}

#[test]
fn a_void_element_keeps_its_attributes() {
    // Not closing it must not lose what it carries — the attributes are the whole content of a
    // void element.
    let h = html_of("    <div><img src=\"a.png\" alt=\"a\"></div>");
    assert!(h.contains("src=\"a.png\""), "attributes survive: {h}");
    assert!(h.contains("alt=\"a\""), "attributes survive: {h}");
    assert!(!h.contains("</img>"), "and it is still not closed: {h}");
}

#[test]
fn the_two_renderers_agree_about_a_void_element() {
    // ★ THE LOAD-BEARING ONE. `html of App` (the server walk) and the MOUNTED page's serializer are
    // two renderers of one view, and `largo build` uses the second while a `-> View` route uses the
    // first. They disagreed about `<input>`, so the same page served two different documents
    // depending on which path produced it.
    let view = "    <form><input id=\"draft\" type=\"text\"></form>";
    let mounted = html_of(view);
    let served = {
        let src = format!("## Component App\n\nView:\n{view}\n\n## Main\nShow html of App.\n");
        let out = tw_outcome(&src);
        assert_eq!(out.error, None, "{src}");
        out.output.trim_end().to_string()
    };
    assert_eq!(
        served.contains("</input>"),
        mounted.contains("</input>"),
        "the served and the mounted renderings must agree:\nserved:  {served}\nmounted: {mounted}"
    );
}
