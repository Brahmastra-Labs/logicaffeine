//! LOCK — a server draws a page with the renderer a browser paints it with.
//!
//! `html of C`, a `## Route … -> View:` template, a `-> View` function's served twin and a page's
//! `Head:` are each drawn by the one view walk (`lower_lgx_nodes`, serving) onto the tape, into a
//! root no host holds, and serialized by `vdSerializeChildren` — the serializer the document itself
//! is held to (`vdom_lg_lock`). There is no second walk building a string and no escaper of the
//! lowering's own: what a server sends and what a browser paints are one document by construction.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

#[path = "support/lgx_lower_source.rs"]
mod lgx_lower_source;

/// `src`'s output on the tree-walker, held equal to the VM's.
fn shown(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM runs it:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "the tiers agree:\n{src}"
    );
    tw.output.trim_end().to_string()
}

/// The generated Rust of `function`'s body, from its signature to the next top-level item — and of
/// every clone the optimizer specialized from it on a constant argument (`function_s<i>_<value>`),
/// each of which is `function` drawn for one caller.
fn rust_fn(rust: &str, function: &str) -> String {
    let mut bodies = String::new();
    let mut from = 0;
    while let Some(at) = rust[from..]
        .find(&format!("fn {function}"))
        .map(|at| from + at)
    {
        let rest = &rust[at..];
        let named = &rest[3 + function.len()..];
        let end = rest[1..].find("\nfn ").map_or(rest.len(), |e| e + 1);
        if named.starts_with('(') || named.starts_with("_s") {
            bodies.push_str(&rest[..end]);
        }
        from = at + end;
    }
    assert!(
        !bodies.is_empty(),
        "the generated Rust has no `{function}`:\n{rust}"
    );
    bodies
}

/// The markup the DOM holds once `component` is mounted — what a browser paints.
fn painted(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "it mounts:\n{src}");
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

const GREETING: &str = "## Component Greeting\nState:\n    Let name be \"<ada & \\\"co\\\">\".\n    Let on be true.\n\
    View:\n    <p class=\"hi\" title={name}>Hi {name}<input disabled={on} value={name}><br></p>\n\n";

#[test]
fn html_of_is_a_detached_render_serialized_by_vd_serialize() {
    let src = format!("{GREETING}## Main\nShow html of Greeting.\n");
    let rust = compile_to_rust(&src).expect("it compiles to Rust");
    let html = rust_fn(&rust, "__html_Greeting");
    for call in [
        "vdTapeStash",
        "vdNewRoot",
        "__serve_Greeting",
        "vdCommit",
        "vdSerializeChildren",
        "vdDiscardRoot",
        "vdTapeUnstash",
    ] {
        assert!(
            html.contains(call),
            "`html of` draws, serializes and discards a root — no `{call}`:\n{html}"
        );
    }
    assert!(
        !html.contains("htmlEscape"),
        "no escaper of the lowering's own:\n{html}"
    );
    let served = shown(&src);
    let mount = painted(&format!("{GREETING}## Main\nMount Greeting.\n"));
    assert_eq!(
        served, mount,
        "the served markup is the painted document, byte for byte"
    );
}

#[test]
fn a_view_route_is_a_detached_render_too() {
    let src = "## Component Card (title: Text)\nView:\n    <li>{title}</li>\n\n\
               ## Route GET /report -> View:\n    <ul><Card title={\"q3 & q4\"}/></ul>\n\n\
               ## Main\nShow route(\"GET\", \"/report\").\n";
    assert_eq!(shown(src), "<ul><li>q3 &amp; q4</li></ul>");
    let rust = compile_to_rust(src).expect("it compiles to Rust");
    assert!(
        rust.contains("vdSerializeChildren"),
        "the route serializes the tape"
    );
    assert!(
        rust_fn(&rust, "__serve_Card").contains("vdOpen"),
        "a placed component draws onto the tape"
    );
}

#[test]
fn a_served_view_function_is_the_view_serialized() {
    let src = "## To badge (label: Text) -> View:\n    <b class=\"badge\">{label}</b>\n\n\
               ## Route GET /b -> View:\n    Return badge(\"<q>\").\n\n\
               ## Main\nShow route(\"GET\", \"/b\").\n";
    assert_eq!(shown(src), "<b class=\"badge\">&lt;q&gt;</b>");
}

/// A page's `Head:` is markup handed to the host whole, drawn and serialized the same way.
#[test]
fn a_head_is_drawn_and_serialized_like_a_served_page() {
    let src = "## Page /\n\nState:\n    Let count be 3.\n\nHead:\n    <title>{count} & counting</title>\n\n\
               View:\n    <p>{count}</p>\n";
    let html = logicaffeine_cli::prerender_page_html(src, "app");
    assert!(
        html.contains("<title>3 &amp; counting</title>"),
        "the head is its serialized markup:\n{html}"
    );
}

#[test]
fn no_second_walk_or_escaper_in_the_lowering() {
    let source = lgx_lower_source::source();
    for gone in [
        "fn lower_view_html",
        "fn lower_component_html",
        "fn html_escape_literal",
        "fn html_escape_text_literal",
        "fn push_html_literal",
    ] {
        assert!(
            !source.contains(gone),
            "the lowering still has `{gone}` — a second server walk"
        );
    }
    for escaper in ["\"htmlEscape\"", "\"htmlEscapeText\""] {
        assert!(
            !source.contains(escaper),
            "the lowering still calls {escaper} — an escaper beside the serializer's"
        );
    }
}

/// `htmlEscape` and `htmlEscapeText` stay the documented functions an author calls, and answer
/// exactly what the serializer writes for an attribute value and for text.
#[test]
fn one_escaper_family() {
    let corpus = [
        "",
        "plain",
        "a&b",
        "<b>",
        "\"quoted\"",
        "'single'",
        "&amp;",
        "Ω ☃ 𝄞",
        "a<b>c&d\"e",
    ];
    let mut main = String::from("## Main\n");
    for s in corpus {
        let lit = s.replace('\\', "\\\\").replace('"', "\\\"");
        main.push_str(&format!(
            "Show htmlEscape(\"{lit}\").\nShow vdEscape(\"{lit}\", true).\n"
        ));
        main.push_str(&format!(
            "Show htmlEscapeText(\"{lit}\").\nShow vdEscape(\"{lit}\", false).\n"
        ));
    }
    let out = shown(&main);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines.len(),
        corpus.len() * 4,
        "every row shows four lines:\n{out}"
    );
    for (i, pair) in lines.chunks(2).enumerate() {
        assert_eq!(
            pair[0], pair[1],
            "row {i}: the documented escaper answers what the serializer writes"
        );
    }
    for s in corpus {
        let attr = logicaffeine_language::lgx::html::escape_attr(s);
        let text = logicaffeine_language::lgx::html::escape_text(s);
        let at = lines.iter().position(|l| *l == attr).unwrap_or(usize::MAX);
        assert!(
            at != usize::MAX,
            "the attribute escape of {s:?} is the document's: {attr}"
        );
        assert!(
            lines.contains(&text.as_str()),
            "the text escape of {s:?} is the document's: {text}"
        );
    }
}

/// A root drawn while another draw is in flight — `html of` in a served component's state —
/// leaves the outer drawing as it was.
#[test]
fn a_root_drawn_inside_another_draw_leaves_it_whole() {
    let src = "## Component Card\nView:\n    <em>card</em>\n\n\
               ## Component Page\nState:\n    Let snippet be html of Card.\nView:\n    <div><p>{snippet}</p><p>after</p></div>\n\n\
               ## Main\nShow html of Page.\n";
    assert_eq!(
        shown(src),
        "<div><p>&lt;em&gt;card&lt;/em&gt;</p><p>after</p></div>"
    );
}

/// ★ REQUIRED: an endless drawing is refused in the render-depth words, never a stack overflow.
#[test]
fn an_endless_html_of_is_refused_in_render_depth_words() {
    let src = "## Component Loop\nView:\n    <div><Loop/></div>\n\n## Main\nShow html of Loop.\n";
    dom::reset();
    let tw = tw_outcome(src);
    let words = logicaffeine_language::ast::component::render_depth_message("Loop");
    assert!(
        tw.error.as_deref().is_some_and(|e| e.contains(&words)),
        "tree-walker: {:?}",
        tw.error
    );
    dom::reset();
    let vm = vm_outcome(src);
    assert!(
        vm.error.as_deref().is_some_and(|e| e.contains(&words)),
        "VM: {:?}",
        vm.error
    );
}

#[test]
fn an_endless_view_is_refused_in_render_depth_words() {
    let src = "## To nest (n: Int) -> View:\n    <div>{nest(n + 1)}</div>\n\n\
               ## Route GET / -> View:\n    <main>{nest(1)}</main>\n\n## Main\nShow route(\"GET\", \"/\").\n";
    dom::reset();
    let tw = tw_outcome(src);
    let words = logicaffeine_language::ast::component::render_depth_message("nest");
    let said = format!("{:?} {}", tw.error, tw.output);
    assert!(said.contains(&words), "tree-walker: {said}");
    dom::reset();
    let vm = vm_outcome(src);
    let said = format!("{:?} {}", vm.error, vm.output);
    assert!(said.contains(&words), "VM: {said}");
}

/// A recursion that stops draws in full, however deep it goes below the limit.
#[test]
fn a_recursion_that_stops_is_drawn_in_full() {
    let src = "## Component Tree (n: Int)\nView:\n    <ul><li>{n}</li>{if n is greater than 1:}<Tree n={n - 1}/>{end}</ul>\n\n\
               ## Component Top\nView:\n    <Tree n={3}/>\n\n## Main\nShow html of Top.\n";
    assert_eq!(
        shown(src),
        "<ul><li>3</li><ul><li>2</li><ul><li>1</li></ul></ul></ul>"
    );
}

/// A server render leaves nothing behind: twenty renders later the retained tree holds what it held
/// before the first.
#[test]
fn serving_again_and_again_retains_nothing() {
    let src = format!(
        "{GREETING}## Main\nLet h0 be html of Greeting.\nLet before be vdLiveNodes().\n\
         Repeat for i from 1 to 20:\n    Let h be html of Greeting.\nShow vdLiveNodes() - before.\n"
    );
    assert_eq!(shown(&src), "0");
}

/// A server render writes the records its markup is made of and nothing a live document needs: no
/// site, instance fragment, listener or anchor — a served page with a handler, a bound field and a
/// placed child carries none of them.
#[test]
fn a_server_render_carries_no_live_document_machinery() {
    let src = "## Component Row (label: Text)\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\n\
               View:\n    <li on-click={bump}>{label} {n}</li>\n\n\
               ## Component App\nState:\n    Let draft be \"hi\".\n    Let n be 1.\nActions:\n    To go:\n        Set n to n + 1.\n\
               View:\n    <div><input bind={draft}><button on-click={go}>{n}</button><Row label=\"a\"/></div>\n\n\
               ## Main\nShow html of App.\n";
    assert_eq!(
        shown(src),
        "<div><input value=\"hi\"><button>1</button><li>a 0</li></div>"
    );
    let rust = compile_to_rust(src).expect("it compiles to Rust");
    for served in ["__serve_App", "__serve_Row"] {
        let body = rust_fn(&rust, served);
        for live in [
            "vdSite",
            "vdListen",
            "vdAnchor",
            "vdFragOpen",
            "__SITE_",
            "__LISTENER",
            "__M_",
            "__S_",
        ] {
            assert!(
                !body.contains(live),
                "`{served}` carries `{live}`, which only a live document needs:\n{body}"
            );
        }
    }
}
