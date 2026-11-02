//! ★ LOCK — an LGX page compiles to WebAssembly. ★
//!
//! This is the whole point of lowering the frontend into DOM builtin calls rather than into new
//! opcodes: those builtins were already lowered to wasm, so a component's render function is
//! ordinary code to this backend and the browser inherits the frontend for free. If that ever
//! stops being true, a LOGOS page stops reaching a browser, and this is the thing that says so.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::compile_to_wasm;

/// One page per construct that has landed on the bytecode tier.
const PAGES: &[(&str, &str)] = &[
    ("element", "## Component App\nView:\n    <div></div>\n\n## Main\nMount App.\n"),
    // `When the user presses "Escape":` — the page-level key trigger. It reaches wasm on the
    // imports that were already frozen (`body`, `addEventListener`, `eventKey`); the listener and
    // the press driver both resolve the page through `querySelector`, which is what makes it
    // answerable rather than merely registered.
    (
        "page-level key press",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When the user presses \"Escape\":\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n\n## Main\nMount App.\n",
    ),
    // `Save` / `Load` / `Forget` — browser storage, reached through the already-frozen
    // `storage_set`/`storage_get`/`storage_remove` imports. The stored text is SELF-DESCRIBING
    // (`i:7`), and the decode is compiled LOGOS on both halves, so no host was involved and the
    // tiers cannot drift apart over what came back out.
    (
        "save, load and forget",
        "## Component App\nState:\n    Let n be 7.\n    Let seen be 0.\nActions:\n    To stash:\n        Save n as \"k\".\n    To drop:\n        Forget \"k\".\n    To fetch:\n        Load \"k\" into seen.\n        Set seen to seen + 1.\nView:\n    <p id=\"s\">{seen}</p><button id=\"a\" on-click={stash}>a</button><button id=\"d\" on-click={drop}>d</button><button id=\"b\" on-click={fetch}>b</button>\n\n## Main\nMount App.\n",
    ),
    // `When a fetch fails (url: Text) and (why: Text):` — the page-wide look at a request that did
    // not work. It rides the imports a typed fetch already used; the failure paths simply were not
    // telling anyone but the action that asked. This is the tier where it MATTERS: a browser really
    // does invoke the completion, so the handler really does run there.
    //
    // ★ THE `Ok` ARM SETS A LITERAL ON PURPOSE. Writing `Set problem to v's text.` — reading a
    // field off the record the answer decoded to — is refused here with "unsupported field access
    // with no static slot". That is a SEPARATE, pre-existing wasm gap (this was the first
    // typed-fetch page ever put to this backend, which is why nothing had said so), and a row that
    // tripped over it would report the fetch-fails handler as broken when it is not.
    (
        "a failed fetch reaches its handler",
        "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n## Component App\nState:\n    Let problem be \"-\".\n    Let failed be \"-\".\nActions:\n    When the page loads:\n        Get \"/api/todo\" as Todo, then loaded.\n    To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n            When Ok (v):\n                Set problem to \"ok\".\n            When Err (e):\n                Set problem to \"err\".\n    When a fetch fails (url: Text) and (why: Text):\n        Set failed to url.\nView:\n    <p id=\"a\">{problem}</p><p id=\"b\">{failed}</p>\n\n## Main\nMount App.\n",
    ),
    // `Set the title to "…".` — one import (`dom_set_title`), and the sentence `docs/web.md`
    // teaches by name stops being one the browser refuses.
    (
        "set the title",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        Set n to n + 1.\n        Set the title to \"Todos\".\nView:\n    <p id=\"n\">{n}</p><button id=\"b\" on-click={go}>b</button>\n\n## Main\nMount App.\n",
    ),
    // `Go back.` — a page-level command only a browser can really perform: one import, and no
    // decision moved out of compiled LOGOS.
    (
        "go back",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        Navigate to \"/todos\".\n    To back:\n        Go back.\nView:\n    <p id=\"p\">{routePath()}</p><button id=\"b\" on-click={go}>go</button><button id=\"k\" on-click={back}>k</button>\n\n## Main\nMount App.\n",
    ),
    // `Add class` / `Remove class` — one READ import, and the rule that decides which class words
    // survive stays in compiled LOGOS that this tier runs too.
    (
        "add and remove a class",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To add:\n        Add class \"act\" to \"#c\".\n    To drop:\n        Remove class \"act\" from \"#c\".\nView:\n    <p id=\"c\" class=\"active\">x</p><button id=\"a\" on-click={add}>a</button><button id=\"d\" on-click={drop}>d</button>\n\n## Main\nMount App.\n",
    ),
    // `When this component disappears:` — a diff between the page and the program's own instance
    // maps, so it needed no host import at all.
    (
        "a component that leaves the page",
        "## Component Row\nState:\n    Let n be 0.\nActions:\n    When this component disappears:\n        Set the title to \"gone\".\nView:\n    <li id=\"r\">row</li>\n\n## Component App\nState:\n    Let show be true.\nActions:\n    To hide:\n        Set show to false.\nView:\n    <ul>{if show:}<Row/>{end}</ul><button id=\"b\" on-click={hide}>b</button>\n\n## Main\nMount App.\n",
    ),
    // The window events — one import for all three, registered on `window` because that is where
    // they fire and they do not bubble.
    (
        "the window events",
        "## Component App\nState:\n    Let off be 0.\n    Let on be 0.\n    Let big be 0.\nActions:\n    When the app goes offline:\n        Set off to off + 1.\n    When the app comes back online:\n        Set on to on + 1.\n    When the window resizes:\n        Set big to big + 1.\nView:\n    <p id=\"s\">{off}{on}{big}</p>\n\n## Main\nMount App.\n",
    ),
    // `When the user scrolls to the bottom:` — one geometry read, threshold in compiled LOGOS.
    (
        "scrolls to the bottom",
        "## Component App\nState:\n    Let page be 0.\n    Let list be 0.\nActions:\n    When the user scrolls to the bottom:\n        Set page to page + 1.\n    When the user scrolls to the bottom of \"#list\":\n        Set list to list + 1.\nView:\n    <div id=\"list\">rows</div><p id=\"s\">{page}{list}</p>\n\n## Main\nMount App.\n",
    ),
    // Timers: one import for the page's clock, and every piece of "which timer is due" compiled.
    (
        "a page that keeps time",
        "## Component App\nState:\n    Let n be 0.\n    Let once be 0.\nActions:\n    Every 10 seconds as refresh:\n        Set n to n + 1.\n    After 25 seconds as settle:\n        Set once to once + 1.\n    To halt:\n        Stop the timer refresh.\nView:\n    <p id=\"n\">{n}</p><p id=\"o\">{once}</p><button id=\"b\" on-click={halt}>b</button>\n\n## Main\nMount App.\n",
    ),
    // `When a file is dropped on …:` — one READ import, and the name travels through the value
    // slot the dispatcher already had.
    (
        "a dropped file",
        "## Component App\nState:\n    Let got be \"-\".\nActions:\n    When a file is dropped on \"#zone\" (name: Text):\n        Set got to name.\nView:\n    <div id=\"zone\">drop here</div><p id=\"g\">{got}</p>\n\n## Main\nMount App.\n",
    ),
    // The element-command family on ONE import: focus and blur (which deliver an event after the
    // turn commits), hover (whose event is `mouseover`, a different word), and scroll/select
    // (which raise nothing at all).
    (
        "the element commands",
        "## Component App\nState:\n    Let log be \"\".\nActions:\n    To go:\n        Focus \"#draft\".\n        Hover \"#card\".\n        Scroll to \"#card\".\n        Select the text in \"#draft\".\n    To stop:\n        Blur \"#draft\".\n    When \"#draft\" is focused:\n        Set log to log + \"F\".\n    When \"#card\" is hovered:\n        Set log to log + \"H\".\nView:\n    <div id=\"card\">c</div><input id=\"draft\"><p id=\"l\">{log}</p><button id=\"a\" on-click={go}>a</button><button id=\"b\" on-click={stop}>b</button>\n\n## Main\nMount App.\n",
    ),
    // The three page capabilities — three imports, because they are three permission models.
    (
        "clipboard, download and notify",
        "## Component App\nState:\n    Let draft be \"notes\".\nActions:\n    To go:\n        Copy \"hello\" to the clipboard.\n        Download draft as \"out.txt\".\n        Notify \"Saved.\".\nView:\n    <button id=\"b\" on-click={go}>b</button>\n\n## Main\nMount App.\n",
    ),
    ("text", "## Component App\nView:\n    <h1>Hello</h1>\n\n## Main\nMount App.\n"),
    (
        "attributes",
        "## Component App\nView:\n    <a href=\"/x\" id=\"go\">link</a>\n\n## Main\nMount App.\n",
    ),
    (
        "state hole",
        "## Component App\nState:\n    Let n be 7.\nView:\n    <p>Count: {n}</p>\n\n## Main\nMount App.\n",
    ),
    (
        "derived",
        "## Component App\nState:\n    Let n be 4.\n    Let doubled always be n * 2.\nView:\n    <p>{doubled}</p>\n\n## Main\nMount App.\n",
    ),
    (
        "conditional",
        "## Component App\nState:\n    Let ready be true.\nView:\n    {if ready:}<p>yes</p>{else:}<p>no</p>{end}\n\n## Main\nMount App.\n",
    ),
    (
        "loop",
        "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs:}<li>{x}</li>{end}</ul>\n\n## Main\nMount App.\n",
    ),
    (
        "loop with an index",
        "## Component App\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x in xs, at index i:}<li>{i}{x}</li>{end}</ul>\n\n## Main\nMount App.\n",
    ),
    (
        "child component",
        "## Component Card\nView:\n    <p>card</p>\n\n## Component App\nView:\n    <div><Card/></div>\n\n## Main\nMount App.\n",
    ),
    (
        "props",
        "## Component Greeting (who: Text)\nView:\n    <p>Hi {who}</p>\n\n## Component App\nView:\n    <Greeting who={\"ada\"}/>\n\n## Main\nMount App.\n",
    ),
    (
        "actions",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n    <button id=\"b\" on-click={bump}>+</button>\n\n## Main\nMount App.\nClick \"#b\".\n",
    ),
    (
        "a When sentence",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#b\" is clicked:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n    <button id=\"b\">+</button>\n\n## Main\nMount App.\nClick \"#b\".\n",
    ),
    (
        "class toggle",
        "## Component App\nState:\n    Let done be true.\nView:\n    <li class=\"row\" class:done={done}>x</li>\n\n## Main\nMount App.\n",
    ),
    (
        "style scoping",
        "## Component App\nView:\n    <p class=\"card\">hi</p>\nStyle:\n    .card { color: red }\n\n## Main\nMount App.\n",
    ),
    (
        "a bound input",
        "## Component App\nState:\n    Let draft be \"\".\nView:\n    <input id=\"d\" bind={draft}>\n    <p>{draft}</p>\n\n## Main\nMount App.\nType \"milk\" into \"#d\".\n",
    ),
    (
        "a bound checkbox",
        "## Component App\nState:\n    Let on be false.\nView:\n    <input id=\"c\" type=\"checkbox\" bind={on}>\n    <p>{on}</p>\n\n## Main\nMount App.\nClick \"#c\".\n",
    ),
    (
        "directive",
        "## Directive tip (el: Element) and (t: Text):\n    Call setAttribute with el and \"title\" and t.\n\n## Component App\nView:\n    <p tip={\"hint\"}>y</p>\n\n## Main\nMount App.\n",
    ),
    (
        "a layout",
        "## Component Site\nView:\n    <main>{block content}</main>\n\n## Component App\nView:\n    {extends Site}\n    {block content:}<p>hi</p>{end}\n\n## Main\nMount App.\n",
    ),
    (
        "children",
        "## Component Card\nView:\n    <div>{children}</div>\n\n## Component App\nView:\n    <Card><p>in</p></Card>\n\n## Main\nMount App.\n",
    ),
    (
        "a page-loads handler",
        "## Component App\nState:\n    Let n be 0.\nActions:\n    When the page loads:\n        Set n to 41.\nView:\n    <p>{n}</p>\n\n## Main\nMount App.\n",
    ),
    (
        "mount at a selector",
        "## Component Shell\nView:\n    <div id=\"root\"></div>\n\n## Component App\nView:\n    <h1>hi</h1>\n\n## Main\nMount Shell.\nMount App at \"#root\".\n",
    ),
    // Stores. A `## Store` is a record global its component's render function reads, which this
    // backend refused until the record-global resolution landed (`wasm_record_global_lock`).
    (
        "store",
        "## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n\n## Component App\nUse Session.\nView:\n    <p>{Session's name}</p>\n\n## Main\nMount App.\n",
    ),
    (
        "store action from a click",
        "## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n\n## Component App\nUse Session.\nActions:\n    To signIn:\n        Call Session's login with \"ada\".\nView:\n    <p id=\"n\">{Session's name}</p>\n    <button id=\"b\" on-click={signIn}>in</button>\n\n## Main\nMount App.\n",
    ),
    // A store that keeps its state between page loads: the fields go out through the host's
    // storage and come back on the next load, which is the whole reason `persisted` is a word.
    (
        "persisted store",
        "## Store Draft persisted as \"draft\"\nState:\n    Let text be \"\".\nActions:\n    To type (t: Text):\n        Set text to t.\n\n## Component App\nUse Draft.\nActions:\n    To fill:\n        Call Draft's type with \"typed\".\nView:\n    <p id=\"t\">{Draft's text}</p>\n    <button id=\"go\" on-click={fill}>x</button>\n\n## Main\nMount App.\n",
    ),
    // ★ A `## Page` — the header a file in `pages/` uses, and the one that reaches a browser most
    // often. It mounts itself, so there is no `## Main` here; that is the point of the entry.
    ("page", "## Page /about\nView:\n    <h1>About</h1>\n"),
    (
        "page with state and an action",
        "## Page /\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n    <button id=\"b\" on-click={bump}>+</button>\n",
    ),
    (
        "page placing a component",
        "## Component Card (label: Text)\nView:\n    <li>{label}</li>\n\n## Page /\nState:\n    Let xs be [\"a\"].\nView:\n    <ul>{for each x in xs:}<Card label={x}/>{end}</ul>\n",
    ),
    (
        "preserved store",
        "## Store Draft preserved across reloads\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\n\n## Component App\nUse Draft.\nActions:\n    To go:\n        Call Draft's bump.\nView:\n    <p id=\"n\">{Draft's n}</p>\n    <button id=\"b\" on-click={go}>+</button>\n\n## Main\nMount App.\n",
    ),
];

#[test]
fn every_landed_construct_compiles_to_wasm() {
    for (name, src) in PAGES {
        let module = compile_to_wasm(src);
        let bytes = match module {
            Ok(b) => b,
            Err(e) => panic!("`{name}` must reach wasm:\n{src}\n=> {e:?}"),
        };
        // ★ VALIDATE, do not merely compile. "The compiler produced bytes" and "those bytes are
        // a module a browser will accept" are different claims, and this lock asserted the weaker
        // one until a page that compiled clean failed to INSTANTIATE with "expected i32, found
        // i64" — an `Int` hole handed straight to a `createTextNode` import that takes a Text
        // handle. Compiling proves the backend did not give up; validating proves it was right.
        if let Err(e) = wasmi::Module::new(&wasmi::Engine::default(), &bytes[..]) {
            panic!("`{name}` compiled but is NOT A VALID MODULE:\n{src}\n=> {e}");
        }
    }
    assert!(
        PAGES.len() >= 41,
        "the wasm page catalog shrank to {} — a construct stopped reaching the browser",
        PAGES.len()
    );
}

#[test]
fn a_page_that_needs_the_runtime_is_refused_by_the_wasm_backend_too() {
    // The refusal is the VM's, and wasm inherits it: one story about what has landed, not two.
    //
    // ★ THE EXAMPLE HAS MOVED TWICE, AND THE SECOND TIME THE WHOLE CATEGORY WENT. First
    // `When the window resizes:` began to compile. Then `When a message arrives on …:` — described
    // right here as "the one trigger deliberately out of scope" — landed as well: the realtime
    // transport it was waiting on turned out to be BUILT (`logicaffeine_system::relay` already
    // carried `Subscribe`/`Publish`/`Event` over one wire protocol), so what was missing was the
    // LGX end rather than a runtime. MEASURED against `ast::component`, the handler gate in
    // `lgx_lower` now allowlists ALL SEVENTEEN `Trigger` variants, so no trigger refuses anywhere.
    //
    // ★ SO THE EXAMPLE IS NO LONGER A TRIGGER, AND NOT A MODIFIER EITHER. Behaviour modifiers
    // (`.stop`, `.once`, `.capture`, `.passive`) now lower too. What stays refused by DESIGN is a
    // bound input whose `type` is computed: the binding's conversion (a checkbox's Bool, a number's
    // Int, anything else's Text) is chosen when the page is compiled, from the `type` the markup
    // writes, so a `type` known only at run time has no conversion to compile. It cannot go stale
    // the way a gap that is merely unimplemented does.
    //
    // `refusal_span_lock` holds the same refusal and asserts WHERE it is reported; this asserts
    // that the wasm backend inherits the refusal at all.
    let src = "## Component App\nState:\n    Let draft be \"\".\n    Let kind be \"text\".\nActions:\n    To go:\n        Set draft to \"\".\n\
               View:\n    <input aria-label=\"d\" type={kind} bind={draft}>\n\n## Main\nMount App.\n";
    let err = compile_to_wasm(src)
        .err()
        .expect("a bound input whose `type` is computed cannot reach wasm");
    let text = format!("{err:?}");
    assert!(
        text.contains("App") || text.contains("tier"),
        "the refusal travels intact: {text}"
    );
    // ★ AND IT IS THE RIGHT REFUSAL. Without this the test passes on ANY failure — a typo in the
    // fixture refuses too, and refuses for a reason that has nothing to do with the tier.
    assert!(
        text.contains("`type` is computed"),
        "the refusal must be the one about the computed `type`, not whatever else the fixture tripped: {text}"
    );
}

/// ★ THE `{inspect}` GAP IS CLOSED FOR THE CASES THAT CAN BE CLOSED, and this test now says so
/// from both sides.
///
/// It used to pin a pure REFUSAL: `{inspect}` lowers to the `Inspect` statement and runs
/// byte-identically on the VM (`lgx_vm_parity_lock`), while the wasm backend could not bind a
/// payload out of the transparent `Option` encoding. The proof that the fault was the BACKEND's
/// rather than the frontend's was that hand-written LOGOS of the same shape failed identically —
/// which is exactly the evidence that said where to fix it. Same shape as the `## Store` gap
/// below, and it closed the same way.
///
/// ★ WHAT CHANGED. `some x` IS `x`, so an `Option` reaches the backend carrying no variant tag.
/// The reference tier's rule is PRESENCE, not tag, and a heap handle can carry that rule exactly:
/// the bump allocator never hands out 0, so `!= 0` is "present". An `Option` of Text, of a record
/// or of a sequence therefore lowers and answers correctly.
///
/// A SCALAR payload cannot, and that boundary is asserted here rather than left to be discovered:
/// `nothing` lowers to the scalar `0`, so `some 0` and `nothing` are the same bits and no test can
/// separate them. Closing that needs a boxed `Option` in the BYTECODE, which the tree-walker and
/// the VM share — an architectural change, not a wasm-local one.
///
/// So this asserts the strictly stronger pair on the same fixtures: the closed case COMPILES on
/// both sides, and the open case REFUSES IDENTICALLY on both sides. The backend-not-frontend
/// property still holds, now proven from the passing direction as well as the failing one.
#[test]
fn an_lgx_inspect_reaches_wasm_exactly_as_plain_logos_does() {
    // CLOSED: a Text payload. Page and plain LOGOS must BOTH reach wasm.
    let page_ok = "## Component App\nState:\n    Let v be some \"hi\".\nView:\n    {inspect v:}{when Some (t):}<p>{t}</p>{when None:}<p>-</p>{end}\n\n## Main\nMount App.\n";
    let plain_ok = "## Main\nLet v be some \"hi\".\nInspect v:\n    When Some (t):\n        Show t.\n    When None:\n        Show \"-\".\n";
    for (what, src) in [("page", page_ok), ("plain", plain_ok)] {
        let bytes = compile_to_wasm(src).unwrap_or_else(|e| {
            panic!("`{what}`: an `Option` of Text must reach wasm — the gap is closed for handle payloads:\n{src}\n=> {e:?}")
        });
        wasmi::Module::new(&wasmi::Engine::default(), &bytes[..]).unwrap_or_else(|e| {
            panic!("`{what}` compiled but is NOT A VALID MODULE:\n{src}\n=> {e}")
        });
    }

    // ★ AND THE SCALAR PAYLOAD, WHICH THIS HALF USED TO PIN AS AN OPEN GAP.
    //
    // `some 3` and `none` were once the same bits in a compiled module, so both the page and the
    // equivalent plain program were refused BY NAME — and this test's job was to prove the two
    // refusals were WORD FOR WORD identical, because a page that failed differently from plain
    // LOGOS would mean the frontend had added a limitation of its own.
    //
    // The bytecode carries a real tag now (`vm/compiler.rs`'s `compile_option_ctor`), so both
    // sides COMPILE. The property under test is unchanged and simply reads from the other
    // direction: the LGX page and the plain program still behave identically, and the frontend
    // still adds nothing. That is the stronger claim, standing on the same two fixtures.
    let page = "## Component App\nState:\n    Let v be some 3.\nView:\n    {inspect v:}{when Some (n):}<p>{n}</p>{when None:}<p>-</p>{end}\n\n## Main\nMount App.\n";
    let plain = "## Main\nLet v be some 3.\nInspect v:\n    When Some (n):\n        Show n.\n    When None:\n        Show 0.\n";
    for (what, src) in [("page", page), ("plain", plain)] {
        let bytes = compile_to_wasm(src).unwrap_or_else(|e| {
            panic!("`{what}`: an `Option` of a scalar reaches wasm now — the tag is real:\n{src}\n=> {e:?}")
        });
        wasmi::Module::new(&wasmi::Engine::default(), &bytes[..]).unwrap_or_else(|e| {
            panic!("`{what}` compiled but is NOT A VALID MODULE:\n{src}\n=> {e}")
        });
    }
}

/// ★ THE STORE GAP IS CLOSED, and this test now says so from the other side.
///
/// It used to pin a REFUSAL: a `## Store` lowers to a record instance in a module-level binding
/// that its component's render FUNCTION reads, and the wasm backend could not resolve a field
/// access on a record GLOBAL from inside a function to a static slot. The proof that the fault
/// was the BACKEND's rather than the frontend's was that hand-written LOGOS of the same shape
/// failed identically while the same record read at TOP LEVEL compiled — which is exactly the
/// evidence that said where to fix it.
///
/// It is fixed (`wasm_record_global_lock` holds it in plain LOGOS, where the gap was reachable
/// from ordinary code). So this now asserts the SAME THREE PROGRAMS COMPILE — the strictly
/// stronger claim, standing on the same fixtures rather than on a gap that no longer exists.
///
/// This was very nearly filed as my own bug. The registry a synthesized type must enter is built
/// from the TOKENS and cloned before parsing, so the store's record does have to be registered in
/// the discovery pass — which it is, and which was necessary but not sufficient. The lesson is
/// that a plausible cause you have just fixed is the easiest thing in the world to mistake for
/// the actual one.
#[test]
fn a_store_reaches_wasm_because_a_record_global_now_resolves_inside_a_function() {
    let store = "## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n\n## Component App\nUse Session.\nView:\n    <p>{Session's name}</p>\n\n## Main\nMount App.\n";
    let hand_written_in_fn = "## A Rec has:\n    a name, which is Text.\n\n## To show_it ():\n    Show r's name.\n\n## Main\nLet r be a new Rec with name \"a\".\nCall show_it.\n";
    let hand_written_top = "## A Rec has:\n    a name, which is Text.\n\n## Main\nLet r be a new Rec with name \"a\".\nShow r's name.\n";

    for (what, src) in [
        ("a page using a store", store),
        ("hand-written LOGOS of the same shape", hand_written_in_fn),
        ("the same record read at top level", hand_written_top),
    ] {
        assert!(
            compile_to_wasm(src).is_ok(),
            "{what} must reach wasm: {:?}",
            compile_to_wasm(src).err()
        );
    }
}

/// A page that `{extends}` a layout AND has an action that sets its own `State:` — the shape every
/// multi-page app with a counter, a form or a toggle takes. The layout's markup and the page's own
/// state each reached wasm alone; together, the module the backend produced must still be one a
/// browser loads.
#[test]
fn an_interactive_page_that_extends_a_layout_compiles_for_the_browser() {
    let src = "## Component Site\nView:\n    <nav><a href=\"/\">Home</a></nav>\n    <main>{block content}</main>\n\n\
               ## Page /contact\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    \
               {extends Site}\n    {block content:}\n      <h1>Contact {n}</h1>\n      <button id=\"go\" on-click={bump}>go</button>\n    {end}\n";
    let bytes = compile_to_wasm(src)
        .unwrap_or_else(|e| panic!("the page must reach wasm:\n{src}\n=> {e:?}"));
    if let Err(e) = wasmi::Module::new(&wasmi::Engine::default(), &bytes[..]) {
        panic!("the page compiled but is NOT A VALID MODULE:\n{src}\n=> {e}");
    }
}
