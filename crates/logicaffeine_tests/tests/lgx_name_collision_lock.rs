//! ★ LOCK — a component may be named after an HTML element, and CSS may be written in any script. ★
//!
//! Two silent wrong answers in the LGX front end, both in byte-wise scanners, both producing
//! something that looks like it worked.
//!
//! ★ 1. `<Link href={u}>Home</Link>` LOSES ITS CHILDREN. The tokenizer asks `is_void(name)` for
//! every start tag, and `Link` lowercases to `link` — a real HTML void element. So the tag is
//! marked self-closing, `Home` is dropped, and the author is then told *"`<Link>` is a void
//! element"* about a component they declared themselves. `Link` is the name every router gives its
//! navigation component; `Input`, `Img`, `Meta`, `Source`, `Track`, `Area`, `Col`, `Embed`, `Base`,
//! `Param`, `Br`, `Hr` and `Wbr` collide the same way.
//!
//! A COMPONENT is not an HTML element — `is_component_name` already draws that line for the rest
//! of the tokenizer (it is how `<TodoRow/>` composes instead of emitting a `<todorow>` tag), and
//! the void check was the one place that did not ask.
//!
//! ★ 2. `Style:` SCOPING MANGLES NON-ASCII. `css::scope`'s selector splitter accumulated with
//! `b[i] as char`, which is Latin-1: every byte of a multi-byte character became its own `char`,
//! so `.café` came out `.cafÃ©`. That is VALID CSS that matches nothing — the rule is emitted, the
//! stylesheet loads, the page renders unstyled, and nothing anywhere is an error. The class in the
//! markup is untouched (it is copied, not scanned), so the two halves of the same name disagree.
//!
//! ★ WHY NEITHER WAS CAUGHT: `lgx_fuzz`'s alphabet was 33 characters and every one was ASCII, so
//! the generator could not reach either. 70,000 cases proved only that the ASCII subset was sound.

use logicaffeine_language::lgx::css;
use logicaffeine_language::lgx::html::{is_component_name, tokenize};

/// A component named after an HTML void element keeps its children.
#[test]
fn a_component_named_after_a_void_element_keeps_its_children() {
    for name in [
        "Link", "Input", "Img", "Meta", "Source", "Track", "Area", "Embed", "Base",
    ] {
        assert!(
            is_component_name(name),
            "`{name}` is a component name by the shared rule"
        );
        let src = format!("<{name} href={{u}}>Home</{name}>");
        let toks = tokenize(&src).unwrap_or_else(|e| panic!("`{src}` must tokenize: {e:?}"));
        let self_closing = toks.iter().any(|t| {
            matches!(t, logicaffeine_language::lgx::html::HtmlToken::StartTag { name: n, self_closing: true, .. } if n == name)
        });
        assert!(
            !self_closing,
            "`<{name}>` was marked self-closing because it lowercases to an HTML void element — \
             its children are dropped and the author is then told their own component is a void \
             element"
        );
    }
}

/// And the real HTML void elements still are void.
#[test]
fn the_actual_void_elements_are_still_void() {
    for name in ["br", "img", "input", "link", "meta", "hr"] {
        let src = format!("<{name}>");
        let toks = tokenize(&src).unwrap_or_else(|e| panic!("`{src}` must tokenize: {e:?}"));
        assert!(
            toks.iter().any(|t| matches!(t,
                logicaffeine_language::lgx::html::HtmlToken::StartTag { name: n, self_closing: true, .. } if n == name)),
            "`<{name}>` is a void element and must stay one"
        );
    }
}

/// A selector in any script survives scoping intact.
#[test]
fn scoping_preserves_non_ascii_selectors() {
    for sel in [".café", ".größe", ".日本", ".naïve span"] {
        let scoped = css::scope(&format!("{sel} {{ color: red; }}"), "lgx-app")
            .unwrap_or_else(|e| panic!("`{sel}` must scope: {e:?}"));
        let core = sel.trim_start_matches('.').split(' ').next().unwrap();
        assert!(
            scoped.contains(core),
            "scoping mangled `{sel}` — the rule is still valid CSS and now matches nothing:\n{scoped}"
        );
        assert!(
            !scoped.contains("Ã"),
            "a multi-byte character was rebuilt one byte at a time (Latin-1):\n{scoped}"
        );
    }
}

/// The scope class still reaches every selector — the mangling fix must not drop the scoping.
#[test]
fn scoping_still_scopes() {
    let scoped = css::scope(".café { color: red; }", "lgx-app").expect("scopes");
    assert!(
        scoped.contains("lgx-app"),
        "the component's scope class is still stamped:\n{scoped}"
    );
}
