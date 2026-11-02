//! ★ LOCK — a page with HANDLERS server-renders, as inert markup. ★
//!
//! ★ THE HIGHEST-VALUE SSR FIX, AND IT IS A DELETION. `on-click={add}` is a child NODE of the
//! element, not an attribute of it, so the server walk met a `ViewNode::Handler` and refused the
//! whole component — which means EVERY INTERACTIVE PAGE. A page with a button was not
//! server-renderable, and a page without one is not much of a page.
//!
//! ★ EMITTING NOTHING FOR IT IS NOT A COMPROMISE — IT IS WHAT SSR IS. Server-rendered markup is
//! inert until hydration: the browser attaches the listeners when the page wakes up, and the
//! server's job is to send the content a reader can see and a crawler can index. A handler has no
//! server-rendered form because it is not content.
//!
//! ★ AND `{inspect}` NEEDED THE OTHER HALF. The walk refused it too, but its EMITTER fell through
//! a `_ => {}` — so accepting it in the gate alone would have rendered the element and silently
//! dropped whichever arm matched. A gate that says yes and an emitter that draws nothing is the
//! "rendered short" failure this whole check exists to prevent, which is why both moved together.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};
use logicaffeine_compile::semantics::dom;

fn html(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.trim().to_string()
}

/// ★ AN INTERACTIVE PAGE SERVER-RENDERS — the markup a reader sees, without the behaviour.
#[test]
fn a_page_with_handlers_renders_its_content_as_inert_markup() {
    let out = html(
        "## Component App\nState:\n\x20   Let n be 0.\n\
         Actions:\n\x20   To add:\n\x20       Set n to n + 1.\n\
         View:\n\x20   <div><p>{n}</p><button on-click={add}>add</button></div>\n\n\
         ## Main\nShow html of App.\n",
    );
    assert!(
        out.contains("<button") && out.contains("add</button>"),
        "the button is CONTENT and must be sent: {out}"
    );
    assert!(
        out.contains("<p>0</p>"),
        "and so is the state it was rendered with: {out}"
    );
    assert!(
        !out.contains("on-click") && !out.contains("onclick"),
        "but the handler is BEHAVIOUR — it has no server-rendered form, and writing one would ship \
         an attribute no browser will honour: {out}"
    );
}

/// ★ `{inspect}` RENDERS THE ARM THAT MATCHES, on the server as in the browser.
///
/// It is `{if}` with a shape to match on, and `{if}` has always server-rendered. Refusing it meant
/// a page that branches on an `Option` — which is every page that shows something that might not be
/// there — could not be sent at all.
#[test]
fn an_inspect_renders_the_arm_that_matches() {
    let present = html(
        "## Component App\nState:\n\x20   Let v be some 3.\n\
         View:\n\x20   {inspect v:}{when Some (n):}<p>got {n}</p>{when None:}<p>-</p>{end}\n\n\
         ## Main\nShow html of App.\n",
    );
    assert!(
        present.contains("<p>got 3</p>"),
        "the matching arm is what renders: {present}"
    );
    assert!(
        !present.contains("<p>-</p>"),
        "and only that arm: {present}"
    );

    let absent = html(
        "## Component App\nState:\n\x20   Let v be none.\n\
         View:\n\x20   {inspect v:}{when Some (n):}<p>got {n}</p>{when None:}<p>-</p>{end}\n\n\
         ## Main\nShow html of App.\n",
    );
    assert!(
        absent.contains("<p>-</p>"),
        "the absent arm renders when nothing is there: {absent}"
    );
    assert!(!absent.contains("got"), "and only that arm: {absent}");
}

/// ★ THE REFUSAL STILL WORKS FOR WHAT IS GENUINELY UNCOVERED, and says which construct.
///
/// Widening a subset is only safe while the edge of it stays loud. `{raw}` is still outside —
/// interpolating unescaped markup is a decision about trust, not a rendering — so it must still
/// refuse by name rather than quietly render nothing.
#[test]
fn a_construct_the_walk_does_not_cover_still_refuses_by_name() {
    dom::reset();
    let err = tw_outcome(
        "## Component App\nState:\n\x20   Let h be \"<b>hi</b>\".\n\
         View:\n\x20   <div>{raw h}</div>\n\n## Main\nShow html of App.\n",
    )
    .error
    .expect("`{raw}` is not server-renderable and must say so");
    assert!(
        err.contains("server-side renderer") || err.contains("server-side walk"),
        "the refusal names the renderer rather than a generated function: {err}"
    );
}

/// ★ A `bind` RENDERS THE FIELD'S CURRENT VALUE — which is what a server-rendered form IS.
///
/// A reader who never loaded the app still sees what is in the fields, and a browser that hydrates
/// finds the same values it would have written. Refusing `bind` shipped every form EMPTY, which
/// looks exactly like a form nobody has filled in.
///
/// Which attribute carries it depends on the element's type — `value` for a text input, the bare
/// `checked` for a checkbox, whose state is its PRESENCE. That question is answered once, in
/// `bound_input_kind`, for both tiers: a checkbox rendered `checked="false"` is one a browser draws
/// TICKED while the page believes it is clear.
#[test]
fn a_bound_field_renders_its_current_value() {
    let text = html(
        "## Component App\nState:\n\x20   Let draft be \"hello\".\n\
         View:\n\x20   <input bind={draft}>\n\n## Main\nShow html of App.\n",
    );
    assert!(
        text.contains("value=\"hello\""),
        "a text input carries its value: {text}"
    );

    let ticked = html(
        "## Component App\nState:\n\x20   Let done be true.\n\
         View:\n\x20   <input type=\"checkbox\" bind={done}>\n\n## Main\nShow html of App.\n",
    );
    assert!(
        ticked.contains("checked"),
        "a ticked checkbox says so: {ticked}"
    );
    assert!(
        !ticked.contains("checked=\"") && !ticked.contains("value="),
        "and says it by PRESENCE — `checked=\"false\"` is a box a browser draws ticked: {ticked}"
    );

    let clear = html(
        "## Component App\nState:\n\x20   Let done be false.\n\
         View:\n\x20   <input type=\"checkbox\" bind={done}>\n\n## Main\nShow html of App.\n",
    );
    assert!(
        !clear.contains("checked"),
        "and a clear one says nothing at all: {clear}"
    );
}

/// ★ `class:` TOGGLES MERGE INTO ONE CLASS ATTRIBUTE, and that is the whole difficulty.
///
/// `class="row"` beside `class:done={d}` is ONE attribute in the document. Written as two, a
/// browser takes the first and discards the rest — so the toggle would silently never apply, and
/// the page would look right in every test that reads the markup as text and wrong in a browser.
#[test]
fn class_toggles_merge_with_the_class_the_author_wrote() {
    let on = html(
        "## Component App\nState:\n\x20   Let d be true.\n\
         View:\n\x20   <li class=\"row\" class:done={d}>x</li>\n\n## Main\nShow html of App.\n",
    );
    assert!(
        on.contains("class=\"row done\""),
        "one attribute, both classes: {on}"
    );
    assert_eq!(
        on.matches("class=").count(),
        1,
        "and exactly one `class=`: {on}"
    );

    let off = html(
        "## Component App\nState:\n\x20   Let d be false.\n\
         View:\n\x20   <li class=\"row\" class:done={d}>x</li>\n\n## Main\nShow html of App.\n",
    );
    assert!(
        off.contains("class=\"row\""),
        "the toggle is absent when it is off: {off}"
    );
    assert!(!off.contains("done"), "and leaves no trace: {off}");
}

/// ★ THE SERVER RENDERER MUST TYPE-CHECK, and AOT-Rust is the only tier that asks.
///
/// The tree-walker and the VM both resolve lazily, so a server-render function they run happily can
/// still be one rustc refuses — and `largo build` goes through rustc. A subject proved on those two
/// tiers alone can be green here and unbuildable in production, which is what
/// `server_surface_deploys_lock` exists to catch.
///
/// ★ IT MATTERS MOST FOR WHAT JUST LANDED. Widening the subset added SHAPES to the emitted
/// function rather than more of the same: an `{inspect}` whose arms BIND names, an attribute whose
/// value is a state field rather than a literal, and a class attribute assembled out of a literal
/// plus guarded appends. Each is a new way for the emitted Rust to be ill-typed, and none existed
/// when this renderer only drew elements, text and holes.
///
/// ★ AND THE SUBJECT IS A `## Route`, NOT `html of`. A program holding a `## Component` is refused
/// by the AOT backend outright — "this program touches the DOM, and the AOT-Rust backend has no
/// document to build" — which is right and is not what this asks about. The deploy-tier use of the
/// server renderer is a route that answers a template, so that is the shape proved here: the same
/// walk, reached the way a server reaches it.
#[test]
fn the_widened_server_subset_compiles_to_rust() {
    let src = "## Route GET /report -> View:\n\
       \x20   {let v be some 3}\n\
       \x20   <div><input value=\"notes\"><li class=\"row\" class:done={true}>x</li>\
       {inspect v:}{when Some (n):}<p>{n}</p>{when None:}<p>-</p>{end}</div>\n\n\
       ## Main\nShow route(\"GET\", \"/report\").\n";

    let rust = compile_to_rust(src).unwrap_or_else(|e| {
        panic!("the server renderer must lower to Rust — largo build goes through rustc: {e:?}")
    });
    assert!(
        rust.contains("fn "),
        "and the route's own render must be in it, not merely the program around it"
    );
}

/// ★ A STYLED COMPONENT MUST SERVE ITS STYLESHEET, or the page ships looking wrong.
///
/// `Style:` scopes its CSS to the component — the browser tier stamps `lgx-<name>` on each
/// selector's subject and emits a `<style>` element as it paints. The server walk drew the markup
/// and not the stylesheet, so a pre-rendered page arrived unstyled and stayed that way until the
/// JavaScript loaded and painted over it.
///
/// ★ THAT IS THE "RENDERED SHORT" FAILURE IN ITS MOST VISIBLE FORM. `largo build` pre-renders every
/// page precisely so a reader sees the real thing before hydration; serving the markup without the
/// rules that lay it out means the one moment pre-rendering exists for is the moment the page looks
/// broken. A refusal would have been better than that, and drawing it is better than both.
#[test]
fn a_styled_component_serves_its_scoped_stylesheet() {
    let out = html(
        "## Component App\nState:\n\x20   Let n be 0.\n\
         View:\n\x20   <div class=\"box\">{n}</div>\n\
         Style:\n\x20   .box { color: red }\n\n\
         ## Main\nShow html of App.\n",
    );
    assert!(
        out.contains("<style"),
        "the stylesheet must be served with the markup: {out}"
    );
    assert!(
        out.contains("color: red"),
        "and carry the author's rules rather than an empty element: {out}"
    );
    assert!(
        out.contains(".box.lgx-app"),
        "SCOPED the way the browser tier scopes it — an unscoped `.box` served beside a scoped \
         `.box.lgx-app` would style different elements on the two tiers: {out}"
    );
    assert!(
        out.contains("class=\"box lgx-app\"") || out.contains("class=\"lgx-app box\""),
        "and the element must carry the scope class, or the rule matches nothing: {out}"
    );
}

/// ★ A WRAPPER COMPONENT SERVES ITS CHILDREN — `<Card><p>hi</p></Card>`.
///
/// A childless placement is a CALL: the server emits `__html_<Child>(props…)` and the child's own
/// function draws itself. A wrapper cannot work that way, because a function has no way to receive
/// NODES — so it is an inlining: the child's view is drawn in place, and where the child writes
/// `{children}` the caller's nodes go instead. That is what the browser tier already does.
///
/// ★ AND IT IS THE SHAPE REAL PAGES ARE MADE OF. A card, a panel, a modal, a layout — the wrapper
/// is how a page stops repeating itself, so refusing it meant the pages most worth pre-rendering
/// were the ones that could not be.
#[test]
fn a_wrapper_component_serves_its_children() {
    let out = html(
        "## Component Card\n\
         View:\n\x20   <div class=\"card\"><header>{title}</header>{children}</div>\n\
         State:\n\x20   Let title be \"Notes\".\n\n\
         ## Component App\n\
         View:\n\x20   <main><Card><p>hi</p></Card></main>\n\n\
         ## Main\nShow html of App.\n",
    );
    assert!(
        out.contains("class=\"card\""),
        "the wrapper's own markup is drawn: {out}"
    );
    assert!(
        out.contains("<header>Notes</header>"),
        "including its own state: {out}"
    );
    assert!(
        out.contains("<p>hi</p>"),
        "and the caller's children go where the slot is: {out}"
    );
    // Order matters: the slot is INSIDE the card, not beside it.
    let card = out.find("class=\"card\"").expect("card");
    let kid = out.find("<p>hi</p>").expect("child");
    let close = out.find("</div>").expect("card closes");
    assert!(
        card < kid && kid < close,
        "the children belong inside the wrapper: {out}"
    );
}

/// ★ AN EMPTY CLASS LIST SERVES NO CLASS ATTRIBUTE, exactly as the browser paints none.
///
/// `<li class:done={done}>` is as ordinary as LGX gets, and when `done` is false the browser
/// writes NO class attribute — its `setAttribute` call sits inside `if list is not "":`. The
/// server wrote the attribute unconditionally, so the same row served `class=""` and painted
/// nothing: the pre-rendered bytes and the hydrated DOM disagreeing about a real page, which is
/// the one thing the server walk exists to prevent.
///
/// ★ FOUND BY BUILDING AN APP, NOT BY A LOCK. Every construct here already had a passing test —
/// the toggle rendered, the wrapper rendered, the scope class rendered. It took putting them in
/// one page and diffing the served markup against the painted DOM for the difference to show,
/// which is why "it renders" and "it renders the SAME" are two different questions.
#[test]
fn a_class_toggle_that_is_off_serves_no_class_attribute() {
    let out = html(
        "## Component Row (label: Text) and (done: Bool)\n\
         View:\n\x20   <li class:done={done}><span>{label}</span></li>\n\n\
         ## Component App\n\
         View:\n\x20   <ul><Row label=\"a\" done={false} /><Row label=\"b\" done={true} /></ul>\n\n\
         ## Main\nShow html of App.\n",
    );
    assert!(
        !out.contains("class=\"\""),
        "an off toggle must serve no class attribute, not an empty one: {out}"
    );
    assert!(
        out.contains("<li><span>a</span></li>"),
        "the off row carries no class: {out}"
    );
    assert!(
        out.contains("<li class=\"done\"><span>b</span></li>"),
        "and the on row still carries its word: {out}"
    );
}

/// Two toggles, so the SEPARATOR is exercised: the space between class words is decided at run
/// time, because whether anything precedes a word depends on the toggles before it. A
/// compile-time space serves `class=" b"` for a row whose first toggle is off and second is on.
#[test]
fn two_toggles_separate_correctly_whichever_are_on() {
    let out = html(
        "## Component Row (a: Bool) and (b: Bool)\n\
         View:\n\x20   <li class:one={a} class:two={b}>x</li>\n\n\
         ## Component App\n\
         View:\n\x20   <ul><Row a={false} b={true} /><Row a={true} b={true} /></ul>\n\n\
         ## Main\nShow html of App.\n",
    );
    assert!(
        out.contains("<li class=\"two\">x</li>"),
        "off-then-on needs no leading space: {out}"
    );
    assert!(
        out.contains("<li class=\"one two\">x</li>"),
        "both on are separated once: {out}"
    );
}

/// ★ A COMPONENT'S STYLESHEET IS NOT EMITTED WHEREVER IT HAPPENS TO BE PLACED.
///
/// A `Style:` block is drawn once per component, at the first placement that renders — and that
/// placement can be anywhere. A styled `Row` inside a `<ul>` put its `<style>` between the `<ul>`
/// and its first `<li>`, which is not a content model any parser accepts: a `<ul>` may contain
/// `<li>` and script-supporting elements, and `<style>` is neither. Browsers tolerate it; a
/// `<table>` does not — the parser FOSTER-PARENTS unexpected content out of the table, relocating
/// the node.
///
/// ★ AND IT IS NOT A RENDERING BUG, WHICH IS WHY IT SURVIVED. The rules apply from anywhere in the
/// document, so every visual test passed. It shows up only when the served bytes are read as
/// HTML — by a validator, by a strict parser, or by a `<table>`.
#[test]
fn a_stylesheet_is_not_emitted_inside_an_element_that_forbids_it() {
    let out = html(
        "## Component Row (label: Text)\n\
         Style:\n\x20   .row { color: red; }\n\
         View:\n\x20   <li class=\"row\">{label}</li>\n\n\
         ## Component App\n\
         View:\n\x20   <ul>{for each x in [\"a\", \"b\"]:}<Row label={x} />{end}</ul>\n\n\
         ## Main\nShow html of App.\n",
    );
    assert!(
        out.contains("<style>"),
        "the stylesheet is still served: {out}"
    );
    assert!(
        !out.contains("<ul><style>"),
        "a `<ul>` may hold only `<li>` and script-supporting elements — a `<style>` there is \
         relocated by a real parser and rejected by a validator: {out}"
    );
    // It belongs ahead of the markup it styles, where `<style>` is always valid.
    let style_at = out.find("<style>").expect("a stylesheet");
    let ul_at = out.find("<ul>").expect("the list");
    assert!(
        style_at < ul_at,
        "the stylesheet precedes the markup it styles: {out}"
    );
}
