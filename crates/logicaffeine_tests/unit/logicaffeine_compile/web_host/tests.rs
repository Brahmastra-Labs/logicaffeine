use super::*;

#[test]
fn browser_host_html_loads_the_module_script() {
    let html = browser_host_html("myapp");
    assert!(html.starts_with("<!doctype html>"), "is an HTML5 document");
    assert!(
        html.contains("<title>myapp</title>"),
        "titled by the app name"
    );
    assert!(
        html.contains("src=\"./myapp.js\""),
        "loads the app's module script"
    );
    assert!(
        html.contains("charset=\"utf-8\"") && html.contains("viewport"),
        "has the base meta"
    );
}

#[test]
fn browser_host_js_wires_the_dom_env() {
    let js = browser_host_js("myapp");
    // The runtime the wasm module imports — a stable env surface.
    for needed in [
        "const env = {",
        "dom_create_element:",
        "dom_add_event_listener:",
        "__logos_call_closure",
        "__logos_render",
        "instance.exports.main();",
    ] {
        assert!(js.contains(needed), "browser host JS must wire `{needed}`");
    }
    assert!(
        js.contains("\"./myapp.wasm\""),
        "fetches the app's wasm by name"
    );
}

#[test]
fn browser_host_js_applies_a_committed_stream_by_its_ids() {
    // G17 pit-of-success guard: a page's paint is reconciled inside the module (`vdom.lg`) and
    // reaches the document as ONE committed stream naming every node by the id it was created
    // under, so a bound <input> the stream does not recreate keeps its node (and focus). The
    // browser's half is applying that stream by those ids; lock its load-bearing surface here.
    // That the applied stream keeps the node is executed, in V8, by `repaint_in_place_lock` and
    // `wasm_dom_browser_lock`.
    let js = browser_host_js("myapp");
    for needed in [
        // The import a commit hands its stream to, decoded as wire v2 and applied by id.
        "dom_apply_words:",
        "decodePatches(wordOperands(",
        "const applyByIds =",
        // A created node takes the id the stream names; a moved one is the same object, inserted
        // before its new neighbour; a freed one gives up its subtree's ids and listeners.
        "const bindNode =",
        "case 'insertBefore':",
        "case 'freeNode': freeSubtree(op[1]); break;",
        // A listener the stream sends again keeps its single DOM listener, updating the flags in
        // place and re-attaching only when capture or passive change.
        "case 'listen':",
        "case 'unlisten':",
        "if (had && attachedAlike(had.flags, flags)) { had.flags = flags; return; }",
    ] {
        assert!(
            js.contains(needed),
            "browser host JS must apply a committed stream by its ids: `{needed}`"
        );
    }
}

#[test]
fn browser_host_js_wires_the_correlated_fetch_contract() {
    // The typed, request-correlated fetch engine: every import of the contract is
    // present, requests ride per-id state (never a shared response global), each
    // request owns an AbortController, and the completion closure receives its
    // OWN reqId through the trampoline.
    let js = browser_host_js("myapp");
    for needed in [
        "dom_fetch_send:",
        "dom_fetch_abort:",
        "dom_fetch_set_timeout:",
        "dom_response_status:",
        "dom_response_body:",
        "dom_response_header:",
        "dom_response_error:",
        "dom_response_text:",
        "new AbortController()",
        "const fetchReqs = new Map()",
        "__logos_call_closure(q.handler, BigInt(id))",
        "FETCH_ALLOWED_HEADERS",
        "'idempotency-key'",
        "'x-expected-version'",
        "FETCH_BODY_CAP = 4194304",
        "'toolarge'",
        "q.timedOut ? 'timeout'",
    ] {
        assert!(js.contains(needed), "browser host JS must wire `{needed}`");
    }
    assert!(
        !js.contains("currentResponse"),
        "the shared-global response body is dead — responses are per-request"
    );
}

#[test]
fn browser_host_js_wires_the_history_location_router() {
    // The router: window.history/location wiring behind the `dom_*` router imports,
    // a popstate listener that trampolines into the LOGOS handler, and settle() on
    // navigation — so back/forward and deep-links drive a re-render.
    let js = browser_host_js("myapp");
    for needed in [
        "dom_route_path:",
        "dom_route_query:",
        "dom_navigate:",
        "dom_replace_route:",
        "dom_on_route:",
        "window.location.pathname",
        "window.history.pushState",
        "window.history.replaceState",
        "new URLSearchParams(window.location.search)",
        // Every route handler the page registered hears a back/forward, in registration order.
        "const routeHandlers = [];",
        "dom_on_route: (handler) => { routeHandlers.push(handler); },",
        "window.addEventListener('popstate', () => { for (const handler of [...routeHandlers]) dispatch(handler, null); });",
    ] {
        assert!(js.contains(needed), "browser host JS must wire `{needed}`");
    }
}

#[test]
fn assemble_preview_embeds_the_wasm_host_and_theme() {
    // The Studio's live preview: an LGX app compiled in-browser to a self-contained
    // iframe srcdoc — wasm inline, the shared runtime, and the theme.
    let src = "## Component App\nState:\n\x20   Let count be 0.\nView:\n\x20   <h1>Count: {count}</h1>\n\n## Main\nMount App.\n";
    let srcdoc = assemble_preview(src).expect("the counter compiles");
    assert!(
        srcdoc.starts_with("<!doctype html>"),
        "a self-contained page"
    );
    assert!(
        srcdoc.contains("new Uint8Array(["),
        "the wasm is embedded inline"
    );
    assert!(
        !srcdoc.contains("fetch(\"./__preview__.wasm\")"),
        "the fetch loader was swapped out"
    );
    assert!(
        srcdoc.contains("dom_create_element"),
        "the DOM runtime is present"
    );
    assert!(
        srcdoc.contains(":root {") && srcdoc.contains("--accent"),
        "the default theme is present"
    );
    assert!(
        srcdoc.contains("instance.exports.main();"),
        "and it runs main()"
    );
    assert!(
        assemble_preview("## Page /\nView:\n\x20   <h1>{oops}</h1>\n").is_err(),
        "a build error surfaces as a ParseError"
    );
}

#[test]
fn error_overlay_shows_the_source_line_caret_and_message() {
    // An undefined hole in a view — a real build error → a themed overlay that shows the
    // offending source, a caret, and the Socratic message (not a bare string).
    let bad = "## Page /\nView:\n\x20   <h1>{oops}</h1>\n";
    let err = crate::compile::compile_to_wasm(bad).expect_err("this program should not compile");
    let overlay = error_overlay_html(&err, bad, "/*reload*/");
    assert!(overlay.contains("LOGOS build error"), "titled");
    assert!(
        overlay.contains("class=\"caret\"") && overlay.contains('^'),
        "shows a caret under the span"
    );
    assert!(overlay.contains("h1"), "shows the offending source line");
    assert!(
        overlay.contains("is not defined"),
        "carries the Socratic message"
    );
    assert!(
        overlay.contains(":root {") && overlay.contains("--accent"),
        "themed with the design tokens"
    );
    assert!(
        overlay.contains("/*reload*/"),
        "embeds the live-reload client"
    );
}

#[test]
fn wasm_host_shim_runs_the_module_under_node() {
    let shim = wasm_host_shim("myapp");
    assert!(shim.contains("readFileSync"), "reads the wasm off disk");
    assert!(shim.contains("\"./myapp.wasm\""), "by the app's name");
    assert!(
        shim.contains("const env = {") && shim.contains("print_i64:"),
        "supplies the print env"
    );
    assert!(shim.contains("instance.exports.main();"), "runs main()");
}
