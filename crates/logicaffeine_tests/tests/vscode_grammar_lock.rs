//! ★ LOCK — the editor grammar knows what an LGX file looks like. ★
//!
//! A `View:` body is real HTML and a `Style:` body is real CSS. Without embedding, an editor
//! lexes `<div class="card">` as LOGOS operators and a stylesheet as gibberish — so the first
//! thing anyone sees of the framework looks broken, and holes inside a template get no
//! highlighting at all.
//!
//! WHAT THIS CAN AND CANNOT SAY, stated plainly rather than implied: a Rust test cannot run a
//! TextMate engine, so it does not prove the highlighting is *pretty*. What it proves is
//! STRUCTURAL and is exactly what silently rots — the file is valid JSON, the embedding rules
//! exist, they name the right foreign scopes, they are ordered before the generic LOGOS patterns
//! (an HTML tag lexed as LOGOS first would never reach them), and every LGX block header is
//! recognised. Those are the properties a careless edit breaks.

use std::path::PathBuf;

fn grammar() -> serde_json::Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../editors/vscode/logicaffeine/syntaxes/logicaffeine.tmLanguage.json");
    let text =
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "the grammar must be valid JSON — an editor silently loads \
                                   nothing when it is not: {e}"
        )
    })
}

fn repo(g: &serde_json::Value, key: &str) -> serde_json::Value {
    g["repository"][key].clone()
}

#[test]
fn the_grammar_is_valid_json() {
    let g = grammar();
    assert_eq!(g["scopeName"], "source.logicaffeine");
    assert!(g["repository"].is_object(), "it must have a repository");
}

#[test]
fn a_view_body_is_embedded_html() {
    let v = repo(&grammar(), "lgx-view");
    assert!(!v.is_null(), "`View:` must have an embedding rule");
    let patterns = v["patterns"]
        .as_array()
        .expect("it includes something")
        .clone();
    let includes: Vec<&str> = patterns
        .iter()
        .filter_map(|p| p["include"].as_str())
        .collect();
    assert!(
        includes.contains(&"text.html.basic"),
        "a `View:` body is real HTML and must embed the HTML grammar, got {includes:?}"
    );
    assert!(
        v["contentName"]
            .as_str()
            .unwrap_or_default()
            .contains("html"),
        "its content scope must say html, so a theme can style it"
    );
}

#[test]
fn a_style_body_is_embedded_css() {
    let v = repo(&grammar(), "lgx-style");
    assert!(!v.is_null(), "`Style:` must have an embedding rule");
    let includes: Vec<&str> = v["patterns"]
        .as_array()
        .expect("it includes something")
        .iter()
        .filter_map(|p| p["include"].as_str())
        .collect();
    assert!(
        includes.contains(&"source.css"),
        "a `Style:` body is real CSS and must embed the CSS grammar, got {includes:?}"
    );
}

#[test]
fn a_hole_inside_a_template_is_highlighted_as_logos() {
    // The whole reason the template syntax is worth having: `{todo's text}` is an ordinary LOGOS
    // expression, and `{if …}` / `{for each …}` are the language rather than a second one. An
    // editor that painted them as HTML text would be teaching the opposite.
    let h = repo(&grammar(), "lgx-hole");
    assert!(!h.is_null(), "a `{{…}}` hole must have a rule");
    let includes: Vec<&str> = h["patterns"]
        .as_array()
        .expect("it includes something")
        .iter()
        .filter_map(|p| p["include"].as_str())
        .collect();
    assert!(
        includes.contains(&"$self"),
        "a hole is LOGOS, so it recurses into the grammar itself, got {includes:?}"
    );
    let view = repo(&grammar(), "lgx-view");
    let view_includes: Vec<&str> = view["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["include"].as_str())
        .collect();
    let hole = view_includes.iter().position(|i| *i == "#lgx-hole");
    let html = view_includes.iter().position(|i| *i == "text.html.basic");
    assert!(
        hole < html,
        "the hole rule must come BEFORE the HTML grammar, or HTML claims the braces first: \
         {view_includes:?}"
    );
}

#[test]
fn the_embedding_rules_come_before_the_generic_logos_patterns() {
    // Order is the whole game in a TextMate grammar. If the generic patterns run first, `<div
    // class="card">` is lexed as operators and a string before the view rule is ever reached, and
    // the embedding never happens at all.
    let g = grammar();
    let tops: Vec<&str> = g["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["include"].as_str())
        .collect();
    let view = tops
        .iter()
        .position(|i| *i == "#lgx-view")
        .expect("the view rule is listed");
    let generic = tops
        .iter()
        .position(|i| *i == "#string" || *i == "#operators" || *i == "#keywords")
        .expect("there are generic patterns");
    assert!(
        view < generic,
        "`#lgx-view` must precede the generic patterns; got {tops:?}"
    );
}

#[test]
fn every_lgx_block_header_is_recognised() {
    // A header nobody highlights reads as prose, which is exactly what these are NOT.
    let g = grammar();
    let bh = serde_json::to_string(&repo(&g, "block-header")).unwrap();
    for header in [
        "Page",
        "Component",
        "Store",
        "Directive",
        "Route",
        "Redirect",
        "Before each request",
        "Error page",
    ] {
        assert!(
            bh.contains(header),
            "`## {header}` is an LGX block header and must be recognised by the grammar"
        );
    }
}

#[test]
fn a_section_ends_at_the_next_section_and_not_at_the_end_of_the_file() {
    // The subtle one. A TextMate grammar cannot see indentation, so the embedded blocks end at a
    // LOOKAHEAD. If that lookahead were missing, `View:` would swallow the rest of the file —
    // including the `Style:` block and every declaration after it.
    let g = grammar();
    for key in ["lgx-view", "lgx-style"] {
        let end = repo(&g, key)["end"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            end.contains("(?="),
            "`{key}` must end at a LOOKAHEAD so the terminating line still belongs to whatever \
             follows; got {end:?}"
        );
        for boundary in ["State", "Actions", "View", "Style", "##"] {
            assert!(
                end.contains(boundary),
                "`{key}` must stop at `{boundary}`, or it swallows the rest of the file: {end:?}"
            );
        }
    }
}

#[test]
fn the_extension_manifest_wires_the_embedded_languages_up() {
    // Declaring the scopes in the grammar is not enough, and the gap is invisible: VS Code will
    // happily PAINT `meta.embedded.block.html` from a theme while never treating it as HTML — so
    // a `View:` body gets no bracket matching, no tag completion, no CSS anything in `Style:`.
    // The grammar would look wired up and be half-connected, which is the shape this whole
    // campaign keeps finding.
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../editors/vscode/logicaffeine/package.json");
    let text =
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
    let manifest: serde_json::Value =
        serde_json::from_str(&text).expect("the extension manifest must be valid JSON");
    let main = manifest["contributes"]["grammars"]
        .as_array()
        .expect("it contributes grammars")
        .iter()
        .find(|g| g["scopeName"] == "source.logicaffeine")
        .expect("the main grammar is contributed");
    let embedded = &main["embeddedLanguages"];
    assert!(
        !embedded.is_null(),
        "the main grammar embeds HTML and CSS, so the manifest has to map those scopes to          languages — otherwise the editor paints them and understands nothing"
    );
    for (scope, lang) in [
        ("meta.embedded.block.html", "html"),
        ("meta.embedded.block.css", "css"),
    ] {
        assert_eq!(
            embedded[scope], lang,
            "`{scope}` must map to `{lang}`; the grammar produces that scope for a `View:`/`Style:` body"
        );
    }
}

#[test]
fn every_scope_the_grammar_produces_for_an_embedded_body_is_mapped() {
    // The two halves held against EACH OTHER rather than against a list I typed twice: any
    // `contentName` the grammar emits for an embedded body must appear in the manifest's map.
    // Adding a third embedded language later cannot silently skip the manifest.
    let g = grammar();
    let mut produced: Vec<String> = Vec::new();
    for key in ["lgx-view", "lgx-style"] {
        if let Some(name) = repo(&g, key)["contentName"].as_str() {
            produced.push(name.to_string());
        }
    }
    assert!(
        !produced.is_empty(),
        "the embedded bodies must name their content scope"
    );

    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../editors/vscode/logicaffeine/package.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let main = manifest["contributes"]["grammars"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["scopeName"] == "source.logicaffeine")
        .unwrap();
    for scope in produced {
        assert!(
            !main["embeddedLanguages"][&scope].is_null(),
            "the grammar produces `{scope}` but the manifest maps no language to it"
        );
    }
}

#[test]
fn a_section_keyword_still_highlights_without_its_embedded_grammar() {
    // ★ AN EMBEDDING THAT CANNOT RESOLVE TAKES THE KEYWORD DOWN WITH IT. `View:` and `Style:` open
    // an EMBEDDED grammar (HTML, CSS), and a TextMate rule whose embedded grammar the editor
    // cannot resolve is dropped WHOLE — so the section keyword lost its own highlighting and read
    // as prose. That is not hypothetical: the repo's own quickguide ratchet tokenizes with a
    // registry that has no `source.css`, and caught `Style:` producing no scope at all.
    //
    // So the plain section rule covers all four names as a FALLBACK, and the embedding rules are
    // listed FIRST: where the embedded grammar IS available they win and the fallback never fires;
    // where it is not, the keyword still reads as a keyword.
    let g = grammar();
    let section = serde_json::to_string(&repo(&g, "lgx-section")).unwrap();
    for name in ["State", "Actions", "View", "Style"] {
        assert!(
            section.contains(name),
            "`{name}:` has a plain fallback, so it highlights even where its embedded grammar is \
             missing: {section}"
        );
    }
    // …and the ORDER that makes the fallback a fallback rather than an override.
    let tops: Vec<&str> = g["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["include"].as_str())
        .collect();
    let at = |n: &str| {
        tops.iter()
            .position(|i| *i == n)
            .unwrap_or_else(|| panic!("{n} is listed"))
    };
    assert!(
        at("#lgx-view") < at("#lgx-section") && at("#lgx-style") < at("#lgx-section"),
        "the EMBEDDING rules come first — otherwise the plain fallback would claim the line and \
         no `View:` body would ever be highlighted as HTML: {tops:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ `Head:` IS A SECTION TOO, AND THE GRAMMAR HAD NEVER HEARD OF IT.
//
// Four of the six example pages in this repository open with a `Head:` block —
// it is where a page's `<title>` and its metadata live, so it is one of the
// first things an author writes and one of the most visible when it reads as
// prose. `State`, `Actions`, `View` and `Style` were all covered; `Head` was
// simply never added, and nothing noticed because a missing highlight does not
// fail anything.
//
// Its body is HTML, exactly like `View:`, so it needs the same treatment: an
// embedding rule, and a place in the plain fallback for editors that cannot
// resolve `text.html.basic`.
// ─────────────────────────────────────────────────────────────────────────────

fn snippets() -> serde_json::Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../editors/vscode/logicaffeine/snippets/logicaffeine.json");
    let text =
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "the snippet file must be valid JSON — VS Code silently loads nothing when it is \
                not: {e}"
        )
    })
}

#[test]
fn the_head_section_is_recognised_like_every_other_section() {
    let g = grammar();
    let section = serde_json::to_string(&repo(&g, "lgx-section")).unwrap();
    assert!(
        section.contains("Head"),
        "`Head:` is a section of a `## Page` and four of this repo's six example pages open with \
         one. Without it the keyword reads as prose: {section}"
    );
}

#[test]
fn a_head_body_is_embedded_html() {
    // `Head:` holds `<title>`, `<meta>`, `<link>` — HTML, the same as `View:`. An editor that
    // does not know that lexes the angle brackets as LOGOS operators.
    let g = grammar();
    let whole = serde_json::to_string(&g).unwrap();
    assert!(
        whole.contains("(Head:)"),
        "the grammar has no rule opening a `Head:` body, so its HTML is lexed as LOGOS"
    );
}

#[test]
fn the_snippets_cover_the_lgx_block_headers() {
    // ★ A SNIPPET SET THAT STOPS AT THE OLD LANGUAGE TEACHES THE OLD LANGUAGE. There were
    // twenty-three snippets and not one of them wrote an LGX block — so the completion list an
    // author browses to learn the surface showed `struct`, `enum` and `theorem` and implied the
    // web framework did not exist.
    let s = snippets();
    let obj = s
        .as_object()
        .expect("the snippet file is an object of named snippets");
    let prefixes: Vec<String> = obj
        .values()
        .filter_map(|v| v["prefix"].as_str().map(str::to_string))
        .collect();
    for want in ["page", "component", "store", "route"] {
        assert!(
            prefixes.iter().any(|p| p == want),
            "there is no `{want}` snippet. An author learning LGX browses this list; it currently \
             offers {prefixes:?}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ A SNIPPET IS THE FIRST PAGE AN AUTHOR WRITES, SO IT HAS TO BE A PAGE THAT WORKS.
//
// The `component` snippet wired its one action twice: a `When "#b" is clicked:` sentence AND an
// `on-click={bump}` directive on the same `#b`. Both are listeners, so one click ran the action
// twice — the counter the snippet teaches counted by two — and nothing in the file looked wrong.
// Snippets are expanded here with their defaults and read through the REAL parser, so the check
// is about what the page means, not how the text is spelled.
// ─────────────────────────────────────────────────────────────────────────────

/// A snippet body with every placeholder replaced by its default: `${1:x}` → `x`, `$1` → the
/// default given for 1, `${1|a,b|}` → `a`, `$0` → nothing.
fn expand_snippet(body: &serde_json::Value) -> String {
    let text = match body {
        serde_json::Value::Array(lines) => lines
            .iter()
            .filter_map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.as_str().unwrap_or_default().to_string(),
    };
    let mut defaults: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(inner) = after.strip_prefix('{') {
            let close = inner.find('}').expect("a `${` placeholder closes");
            let (num, default) = match inner[..close].split_once(':') {
                Some((n, d)) => (n.to_string(), d.to_string()),
                None => {
                    let (n, choices) = inner[..close]
                        .split_once('|')
                        .unwrap_or((&inner[..close], ""));
                    (
                        n.to_string(),
                        choices
                            .trim_end_matches('|')
                            .split(',')
                            .next()
                            .unwrap_or("")
                            .to_string(),
                    )
                }
            };
            defaults.insert(num, default.clone());
            out.push_str(&default);
            rest = &inner[close + 1..];
        } else {
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            out.push_str(defaults.get(&digits).map(String::as_str).unwrap_or(""));
            rest = &after[digits.len()..];
        }
    }
    out.push_str(rest);
    out
}

/// Every snippet that writes an LGX component or page, as `(snippet name, expanded source)`.
fn lgx_view_snippets() -> Vec<(String, String)> {
    let s = snippets();
    let found: Vec<(String, String)> = s
        .as_object()
        .expect("the snippet file is an object of named snippets")
        .iter()
        .map(|(name, v)| (name.clone(), expand_snippet(&v["body"])))
        .filter(|(_, src)| src.contains("## Component") || src.contains("## Page"))
        .collect();
    assert!(
        found.len() >= 2,
        "the `page` and `component` snippets write views; found {:?}",
        found.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    found
}

/// The `(selector, event)` pairs a parsed program answers, twice over: from `When "…" is …:`
/// sentences, and from `on-<event>` directives on an element with a static `id`.
fn wired_events(src: &str) -> (Vec<(String, String)>, Vec<(String, String)>) {
    use logicaffeine_language::ast::component::Trigger;
    use logicaffeine_language::ast::stmt::{Expr, Literal, Stmt};
    use logicaffeine_language::ast::view::{Attr, ViewNode};

    fn walk(
        nodes: &[ViewNode<'_>],
        interner: &logicaffeine_base::Interner,
        out: &mut Vec<(String, String)>,
    ) {
        for n in nodes {
            match n {
                ViewNode::Element {
                    attrs, children, ..
                } => {
                    let id = attrs.iter().find_map(|a| match a {
                        Attr::Plain {
                            name,
                            value: Expr::Literal(Literal::Text(v)),
                        } if interner.resolve(*name) == "id" => {
                            Some(interner.resolve(*v).to_string())
                        }
                        _ => None,
                    });
                    for c in children {
                        if let (Some(id), ViewNode::Handler { event, .. }) = (&id, c) {
                            out.push((format!("#{id}"), interner.resolve(*event).to_string()));
                        }
                    }
                    walk(children, interner, out);
                }
                ViewNode::Component { children, .. } => walk(children, interner, out),
                _ => {}
            }
        }
    }

    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.unwrap_or_else(|e| panic!("the snippet must parse: {e}\n{src}"));
        let mut sentences = Vec::new();
        let mut directives = Vec::new();
        for s in stmts {
            let Stmt::ComponentDef(def) = s else { continue };
            for handler in &def.events {
                if let Trigger::Selector {
                    selector: Expr::Literal(Literal::Text(sel)),
                    event,
                    ..
                } = &handler.trigger
                {
                    sentences.push((
                        interner.resolve(*sel).to_string(),
                        interner.resolve(*event).to_string(),
                    ));
                }
            }
            if let Some(view) = &def.view {
                walk(&view.nodes, interner, &mut directives);
            }
        }
        (sentences, directives)
    })
}

#[test]
fn no_snippet_wires_one_elements_event_twice() {
    for (name, src) in lgx_view_snippets() {
        let (sentences, directives) = wired_events(&src);
        let twice: Vec<&(String, String)> = sentences
            .iter()
            .filter(|p| directives.contains(p))
            .collect();
        assert!(
            twice.is_empty(),
            "the `{name}` snippet answers {twice:?} with BOTH a `When … is …:` sentence and an \
             `on-…` directive — each is a listener, so one event runs the action twice:\n{src}"
        );
    }
}

#[test]
fn the_double_wiring_check_sees_both_forms_of_listener() {
    // Without this the check above passes on a parser that reports nothing: both forms are
    // written here, on one element, and both must be seen.
    let src = "## Component Counter\nState:\n    Let count be 0.\nActions:\n    To bump:\n        \
               Set count to count + 1.\n    When \"#b\" is clicked:\n        Set count to count + 1.\n\
               View:\n    <button id=\"b\" on-click={bump}>+</button>\n";
    let (sentences, directives) = wired_events(src);
    let pair = ("#b".to_string(), "click".to_string());
    assert!(
        sentences.contains(&pair),
        "the sentence is read as a click on #b: {sentences:?}"
    );
    assert!(
        directives.contains(&pair),
        "the directive is read as a click on #b: {directives:?}"
    );
}

#[test]
fn every_view_snippet_builds_for_the_browser() {
    for (name, src) in lgx_view_snippets() {
        if let Err(e) = logicaffeine_compile::compile::compile_to_wasm(&src) {
            panic!(
                "the `{name}` snippet expands to a page the browser build refuses — a snippet \
                 teaches the page an author ships: {}\n{src}",
                logicaffeine_language::socratic_explanation(
                    &e,
                    &logicaffeine_base::Interner::new()
                )
            );
        }
    }
}
