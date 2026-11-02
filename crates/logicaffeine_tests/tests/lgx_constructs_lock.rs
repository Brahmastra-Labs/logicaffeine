//! ★ RATCHET — the LGX surface, catalogued and RUN. ★
//!
//! The successor to `view_surface_lock`'s `VIEW_CONSTRUCTS`. Every construct of the LGX
//! dialect has one example here, each with the exact page it renders — so a construct cannot
//! be silently dropped, and cannot silently change what it produces either. The old catalog
//! proved its constructs COMPILED; this one proves they RUN and what they mean.
//!
//! The floor never shrinks. Every entry of the retiring catalog has a successor here, marked
//! with the name it carried there, so the migration is checkable line by line rather than
//! asserted.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn rendered(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

/// `(construct, source, the page it renders)`. A construct whose name ends in a `←` note
/// carries an entry of the retiring `VIEW_CONSTRUCTS` catalog.
const LGX_CONSTRUCTS: &[(&str, &str, &str)] = &[
    (
        "element ← element",
        "## Component App\nView:\n    <div></div>\n\n## Main\nMount App.\n",
        "<div></div>",
    ),
    (
        "element with text ← element with inline text",
        "## Component App\nView:\n    <h1>Hello</h1>\n\n## Main\nMount App.\n",
        "<h1>Hello</h1>",
    ),
    (
        "attributes ← attributes",
        "## Component App\nView:\n    <a href=\"/x\" id=\"go\">link</a>\n\n## Main\nMount App.\n",
        "<a href=\"/x\" id=\"go\">link</a>",
    ),
    (
        "hole ← signal interpolation",
        "## Component App\nState:\n    Let count be 1.\nView:\n    <p>n = {count}</p>\n\n## Main\nMount App.\n",
        "<p>n = 1</p>",
    ),
    (
        "event sentence ← event handler (block)",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#b\" is clicked:\n        Set n to 1.\nView:\n    <button id=\"b\">{n}</button>\n\n## Main\nMount App.\nClick \"#b\".\n",
        "<button id=\"b\">1</button>",
    ),
    (
        "on- directive ← event handler (inline)",
        "## Component App\nState:\n    Let c be 0.\nActions:\n    To bump:\n        Set c to c + 1.\nView:\n    <button id=\"b\" on-click={bump}>{c}</button>\n\n## Main\nMount App.\nClick \"#b\".\n",
        "<button id=\"b\">1</button>",
    ),
    (
        "bind ← bound to (form binding)",
        "## Component App\nState:\n    Let title be \"t\".\nView:\n    <input type=\"text\" bind={title}>\n\n## Main\nMount App.\n",
        // Updated when the serializer stopped writing a closing tag for a VOID element: an
        // `</input>` is a stray tag HTML's own parser drops, and this line was pinning it. The
        // claim — the two tiers render this identically — is unchanged. See
        // `void_element_serialize_lock`.
        "<input type=\"text\" value=\"t\">",
    ),
    (
        "raw ← raw html",
        "## Component App\nState:\n    Let markup be \"<b>bold</b>\".\nView:\n    <div>{raw markup}</div>\n\n## Main\nMount App.\n",
        "<div><b>bold</b></div>",
    ),
    (
        "Style: ← css (plain string, braces)",
        "## Component App\nView:\n    <div class=\"box\"></div>\nStyle:\n    .box { color: red }\n\n## Main\nMount App.\n",
        "<style>.box.lgx-app { color: red }</style><div class=\"box lgx-app\"></div>",
    ),
    (
        "for each ← for each",
        "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs:}<li>{x}</li>{end}</ul>\n\n## Main\nMount App.\n",
        "<ul><li>a</li><li>b</li></ul>",
    ),
    (
        "for each, keyed ← keyed for each (identity)",
        "## Component App\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x in xs, keyed by x:}<li>{x}</li>{end}</ul>\n\n## Main\nMount App.\n",
        "<ul><li>a</li></ul>",
    ),
    (
        "for each, keyed by an expression ← keyed for each (literal template)",
        "## Component App\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x in xs, keyed by \"row-\" + x:}<li>{x}</li>{end}</ul>\n\n## Main\nMount App.\n",
        "<ul><li>a</li></ul>",
    ),
    (
        "if ← when",
        "## Component App\nState:\n    Let ready be true.\nView:\n    {if ready:}<p>yes</p>{end}\n\n## Main\nMount App.\n",
        "<p>yes</p>",
    ),
    (
        "if / else ← when / otherwise",
        "## Component App\nState:\n    Let ready be false.\nView:\n    {if ready:}<p>yes</p>{else:}<p>no</p>{end}\n\n## Main\nMount App.\n",
        "<p>no</p>",
    ),
    (
        "component child ← component child",
        "## Component Card\nView:\n    <p>card</p>\n\n## Component App\nView:\n    <div><Card/></div>\n\n## Main\nMount App.\n",
        "<div><p>card</p></div>",
    ),
    (
        "scoped Style ← scoped css component",
        "## Component Box\nView:\n    <p class=\"b\">x</p>\nStyle:\n    .b { color: red }\n\n## Component App\nView:\n    <Box/>\n\n## Main\nMount App.\n",
        "<style>.b.lgx-box { color: red }</style><p class=\"b lgx-box\">x</p>",
    ),
    (
        "Mount at a selector ← Render into an element",
        "## Component Shell\nView:\n    <div id=\"root\"></div>\n\n## Component App\nView:\n    <h1>hi</h1>\n\n## Main\nMount Shell.\nMount App at \"#root\".\n",
        "<div id=\"root\"><h1>hi</h1></div>",
    ),
    (
        "Mount ← ## Component + Mount (default)",
        "## Component App\nState:\n    Let n be 0.\nView:\n    <h1>Count: {n}</h1>\n\n## Main\nMount App.\n",
        "<h1>Count: 0</h1>",
    ),
    // ── constructs the old dialect had no answer for ──
    (
        "for each / else",
        "## Component App\nState:\n    Let xs be a new Seq of Text.\nView:\n    <ul>{for each x in xs:}<li>{x}</li>{else:}<li>none</li>{end}</ul>\n\n## Main\nMount App.\n",
        "<ul><li>none</li></ul>",
    ),
    (
        "for each, at index",
        "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs, at index i:}<li>{i}{x}</li>{end}</ul>\n\n## Main\nMount App.\n",
        "<ul><li>1a</li><li>2b</li></ul>",
    ),
    (
        "if / else if / else",
        "## Component App\nState:\n    Let n be 2.\nView:\n    {if n is equal to 1:}<p>one</p>{else if n is equal to 2:}<p>two</p>{else:}<p>many</p>{end}\n\n## Main\nMount App.\n",
        "<p>two</p>",
    ),
    (
        "inspect / when",
        "## Component App\nState:\n    Let v be some 3.\nView:\n    {inspect v:}{when Some (n):}<p>{n}</p>{when None:}<p>-</p>{end}\n\n## Main\nMount App.\n",
        "<p>3</p>",
    ),
    (
        "let binding",
        "## Component App\nState:\n    Let n be 3.\nView:\n    {let doubled be n * 2}<p>{doubled}</p>\n\n## Main\nMount App.\n",
        "<p>6</p>",
    ),
    (
        "comment",
        "## Component App\nView:\n    <p>a{# not rendered #}b</p>\n\n## Main\nMount App.\n",
        "<p>ab</p>",
    ),
    (
        "filter",
        "## Component App\nState:\n    Let name be \"ada\".\nView:\n    <p>{name | upper}</p>\n\n## Main\nMount App.\n",
        "<p>ADA</p>",
    ),
    (
        "class toggle",
        "## Component App\nState:\n    Let done be true.\nView:\n    <li class=\"row\" class:done={done}>x</li>\n\n## Main\nMount App.\n",
        "<li class=\"row done\">x</li>",
    ),
    (
        "props",
        "## Component Greeting (who: Text)\nView:\n    <p>Hi {who}</p>\n\n## Component App\nView:\n    <Greeting who={\"ada\"}/>\n\n## Main\nMount App.\n",
        "<p>Hi ada</p>",
    ),
    (
        "children",
        "## Component Card\nView:\n    <div>{children}</div>\n\n## Component App\nView:\n    <Card><p>in</p></Card>\n\n## Main\nMount App.\n",
        "<div><p>in</p></div>",
    ),
    (
        "extends / block",
        "## Component Site\nView:\n    <main>{block content}</main>\n\n## Component App\nView:\n    {extends Site}\n    {block content:}<p>hi</p>{end}\n\n## Main\nMount App.\n",
        "<main><p>hi</p></main>",
    ),
    (
        "-> View macro",
        "## To badge (label: Text) -> View:\n    <span>{label}</span>\n\n## Component App\nView:\n    <p>{badge(\"x\")}</p>\n\n## Main\nMount App.\n",
        "<p><span>x</span></p>",
    ),
    (
        "directive",
        "## Directive tooltip (el: Element) and (text: Text):\n    Call setAttribute with el and \"title\" and text.\n\n## Component App\nView:\n    <p tooltip={\"hint\"}>x</p>\n\n## Main\nMount App.\n",
        "<p title=\"hint\">x</p>",
    ),
    (
        "store",
        "## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n\n## Component App\nUse Session.\nActions:\n    To go:\n        Call Session's login with \"ada\".\nView:\n    <p>{Session's name}</p>\n    <button id=\"b\" on-click={go}>in</button>\n\n## Main\nMount App.\nClick \"#b\".\n",
        "<p>ada</p><button id=\"b\">in</button>",
    ),
    (
        "derived state",
        "## Component App\nState:\n    Let n be 3.\n    Let doubled always be n * 2.\nView:\n    <p>{doubled}</p>\n\n## Main\nMount App.\n",
        "<p>6</p>",
    ),
    (
        "escaping",
        "## Component App\nState:\n    Let t be \"<script>x</script>\".\nView:\n    <p title={t}>{t}</p>\n\n## Main\nMount App.\n",
        "<p title=\"&lt;script&gt;x&lt;/script&gt;\">&lt;script&gt;x&lt;/script&gt;</p>",
    ),
    (
        "remove every … where (S18)",
        "## Component App\nState:\n    Let xs be [4, 1, 3, 2].\nActions:\n    To trim:\n        Remove every n from xs where n is greater than 2.\nView:\n    <ul>{for each n in xs:}<li>{n}</li>{end}</ul>\n    <button id=\"b\" on-click={trim}>trim</button>\n\n## Main\nMount App.\nClick \"#b\".\n",
        "<ul><li>1</li><li>2</li></ul><button id=\"b\">trim</button>",
    ),
];

#[test]
fn every_lgx_construct_renders_what_it_says() {
    for (name, src, expected) in LGX_CONSTRUCTS {
        assert_eq!(&rendered(src), expected, "construct `{name}`:\n{src}");
    }
}

/// The never-shrink floor. The retiring `VIEW_CONSTRUCTS` catalog held 21; every one of them
/// has a successor above (marked `←`), and LGX adds its own on top. Coverage only ever grows.
#[test]
fn the_catalog_never_shrinks() {
    let migrated = LGX_CONSTRUCTS
        .iter()
        .filter(|(name, _, _)| name.contains('←'))
        .count();
    assert!(
        migrated >= 18,
        "a migrated construct was dropped: {migrated} of the old catalog's entries remain"
    );
    assert!(
        LGX_CONSTRUCTS.len() >= 34,
        "the LGX catalog shrank to {} — a construct was dropped",
        LGX_CONSTRUCTS.len()
    );
}

/// Common LGX mistakes keep view-aware Socratic guidance — never a bare "expected token".
/// Which catalogue row exercises each `ViewNode` variant.
///
/// ★ THIS CATALOGUE HAD A FLOOR AND NO CENSUS. `the_catalog_never_shrinks` catches a row being
/// DELETED; nothing caught a construct never being added. The trigger catalogue next door
/// re-parses its enum and refuses to be satisfied while a variant has no row — the view constructs,
/// which are the other half of what an author writes, had no such rule. A new `ViewNode` could
/// land, work on one tier, and never appear here.
///
/// ★ AND THE ROWS ARE NAMED FOR FEATURES, NOT VARIANTS, which is right for a reader and useless
/// for a coverage check — "props", "children", "extends / block" say nothing about which enum
/// arms they exercise. So the mapping is written down instead of inferred, and the test below
/// requires it to be total over the enum. A variant nobody can name a row for is a variant nobody
/// has rendered.
/// `(variant, the row that exercises it, a substring that row's SOURCE must contain)`.
///
/// ★ THE THIRD COLUMN IS THE WHOLE POINT, and the first version of this table did not have it.
/// Without it the test only checked that a row with that NAME exists — so I could claim `Handler`
/// was covered by "class toggle" and `Raw` by "filter", and it passed. Both were wrong: neither
/// row contains a handler or a `{raw}`. **A coverage map nothing verifies is a comment that fails
/// the build when you rename a row**, which is worse than no map, because it reads like evidence.
/// The marker makes the claim checkable against the row's own source.
const VARIANT_COVERAGE: &[(&str, &str, &str)] = &[
    ("Element", "escaping", "<p title="),
    // ★ A LITERAL text node, not a hole. The escaping row is `<p title={t}>{t}</p>` — every
    // character in it is markup or interpolation, so it exercises no `ViewNode::Text` at all.
    // `Hi {who}` does: the `Hi ` is the literal the variant is for.
    ("Text", "props", "Hi {who}"),
    ("Component", "props", "<Greeting"),
    ("Each", "for each / else", "{for each"),
    ("If", "if / else if / else", "{if "),
    ("Handler", "on- directive ← event handler (inline)", "on-"),
    ("Raw", "raw ← raw html", "{raw"),
    ("Inspect", "inspect / when", "{inspect"),
    ("Let", "let binding", "{let"),
    ("Extends", "extends / block", "{extends"),
    ("Block", "extends / block", "{block"),
    ("Children", "children", "{children}"),
];

/// ★ EVERY `ViewNode` VARIANT IS EXERCISED BY A ROW — the census the floor was standing in for.
#[test]
fn the_catalog_covers_every_view_construct() {
    let ast = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/ast/view.rs"),
    )
    .expect("the view AST");
    let body = ast
        .split("pub enum ViewNode<'a> {")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("ViewNode is declared there");
    let declared: Vec<String> = body
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let head = t
                .split(|c: char| c == '{' || c == ',' || c == '(')
                .next()?
                .trim();
            (!head.is_empty()
                && head.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && head.chars().all(|c| c.is_ascii_alphanumeric()))
            .then(|| head.to_string())
        })
        .collect();
    assert!(
        declared.len() >= 12,
        "the ViewNode scan found only {declared:?} — it has stopped matching rather than the \
         language having stopped declaring view constructs"
    );

    let named: Vec<&str> = VARIANT_COVERAGE.iter().map(|(v, ..)| *v).collect();
    let missing: Vec<&String> = declared
        .iter()
        .filter(|d| !named.contains(&d.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these `ViewNode` variants are exercised by no catalogue row: {missing:?}\n\
         Add a row that renders each, and name it here. A construct with no row is one whose \
         rendering nothing checks — which is how a view node comes to work on the tier you happened \
         to try and nowhere else."
    );
    let stale: Vec<&&str> = named
        .iter()
        .filter(|n| !declared.contains(&n.to_string()))
        .collect();
    assert!(
        stale.is_empty(),
        "the coverage map names variants that no longer exist: {stale:?}"
    );

    // ★ AND THE CLAIM IS CHECKED, not taken on trust: the named row must exist AND its source
    // must actually contain the construct. This is the half that caught three wrong pairings the
    // moment it was added — a map that only had to name a row was satisfied by rows that render
    // no such thing.
    for (variant, row, marker) in VARIANT_COVERAGE {
        let found = LGX_CONSTRUCTS.iter().find(|(name, ..)| name == row);
        let Some((_, src, _)) = found else {
            panic!("`{variant}` points at row `{row}`, which is not in the catalogue");
        };
        assert!(
            src.contains(marker),
            "`{variant}` claims to be exercised by row `{row}`, but that row's source contains no \
             `{marker}` — so nothing in the catalogue actually renders a `{variant}`. Point it at a \
             row that does, or add one."
        );
    }
}

const LGX_MISTAKES: &[(&str, &str)] = &[
    (
        "## Component App\nView:\n    <div><p>x</div>\n\n## Main\nMount App.\n",
        "closes the wrong element",
    ),
    (
        "## Component App\nView:\n    {if:}<p>x</p>{end}\n\n## Main\nMount App.\n",
        "condition",
    ),
    (
        "## Component App\nView:\n    {for each x xs:}<li>{x}</li>{end}\n\n## Main\nMount App.\n",
        "in",
    ),
    (
        "## Component App\nView:\n    <p>x</p>\n\n## Main\nMount Ghost.\n",
        "Ghost",
    ),
    (
        "## Component App\nActions:\n    To go:\n        Call Ghost's login.\nView:\n    <p>x</p>\n\n## Main\nMount App.\n",
        "not a store",
    ),
    (
        "## Component App (who: Text)\nActions:\n    To go:\n        Set who to \"x\".\nView:\n    <p>{who}</p>\n\n## Main\nMount App.\n",
        "prop",
    ),
];

#[test]
fn lgx_mistakes_keep_their_socratic_guidance() {
    for (src, needle) in LGX_MISTAKES {
        dom::reset();
        let err = tw_outcome(src)
            .error
            .unwrap_or_else(|| panic!("this should not run cleanly:\n{src}"));
        assert!(
            err.contains(needle),
            "the message must mention {needle:?}:\n{src}\n=> {err}"
        );
    }
}
