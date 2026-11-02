//! ★ LOCK — layouts and slots: `{extends}`, `{block}`, `{children}`. ★
//!
//! A layout is just a component with `{block}` slots, and a page fills them by extending it.
//! The rule that makes this composable rather than confusing: slot content is rendered where it
//! was WRITTEN, in the scope it was written in. A page's `{block content:}` reads the page's
//! state even though it lands inside the layout, and a `<Comp>…</Comp>` child reads the
//! caller's bindings even though it lands inside the component.

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

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

fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

#[test]
fn a_page_renders_inside_the_layout_it_extends() {
    let src = "\
## Component Site
View:
    <header>nav</header>
    <main>{block content}</main>

## Component App
View:
    {extends Site}
    {block content:}<p>hello</p>{end}

## Main
Mount App.
";
    assert_eq!(
        rendered(src),
        "<header>nav</header><main><p>hello</p></main>"
    );
}

#[test]
fn an_unfilled_block_falls_back_to_the_layouts_default() {
    let src = "\
## Component Site
View:
    <title>{block title:}LOGOS{end}</title>
    <main>{block content}</main>

## Component App
View:
    {extends Site}
    {block content:}<p>hello</p>{end}

## Main
Mount App.
";
    assert_eq!(
        rendered(src),
        "<title>LOGOS</title><main><p>hello</p></main>"
    );
}

#[test]
fn a_filled_block_wins_over_the_default() {
    let src = "\
## Component Site
View:
    <title>{block title:}LOGOS{end}</title>

## Component App
View:
    {extends Site}
    {block title:}Todos{end}

## Main
Mount App.
";
    assert_eq!(rendered(src), "<title>Todos</title>");
}

#[test]
fn block_content_reads_the_pages_state_not_the_layouts() {
    let src = "\
## Component Site
State:
    Let name be \"layout\".
View:
    <main>{block content}</main>
    <footer>{name}</footer>

## Component App
State:
    Let name be \"page\".
View:
    {extends Site}
    {block content:}<p>{name}</p>{end}

## Main
Mount App.
";
    assert_eq!(
        rendered(src),
        "<main><p>page</p></main><footer>layout</footer>"
    );
}

#[test]
fn a_components_children_render_where_it_says() {
    let src = "\
## Component Card
View:
    <div class=\"card\">{children}</div>

## Component App
View:
    <Card><p>inside</p></Card>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<div class=\"card\"><p>inside</p></div>");
}

#[test]
fn children_read_the_callers_bindings() {
    let src = "\
## Component Card
State:
    Let label be \"card\".
View:
    <div>{children}</div>

## Component App
State:
    Let label be \"app\".
View:
    <Card><p>{label}</p></Card>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<div><p>app</p></div>");
}

#[test]
fn a_component_with_no_children_renders_nothing_at_its_slot() {
    let src = "\
## Component Card
View:
    <div>[{children}]</div>

## Component App
View:
    <Card/>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<div>[]</div>");
}

#[test]
fn a_loop_may_supply_children_and_each_keeps_its_own_item() {
    let src = "\
## Component Card
View:
    <li>{children}</li>

## Component App
State:
    Let items be [\"a\", \"b\"].
View:
    <ul>{for each x in items:}<Card><b>{x}</b></Card>{end}</ul>

## Main
Mount App.
";
    assert_eq!(rendered(src), "<ul><li><b>a</b></li><li><b>b</b></li></ul>");
}

#[test]
fn extending_a_component_that_does_not_exist_says_so() {
    let src = "## Component App\nView:\n    {extends Ghost}\n    {block content:}<p>x</p>{end}\n\n## Main\nMount App.\n";
    let msg = failure(src);
    assert!(msg.contains("Ghost"), "the message names the layout: {msg}");
    assert!(
        msg.contains("## Component Ghost"),
        "and shows how to declare it: {msg}"
    );
}

#[test]
fn a_layout_may_carry_its_own_style_scope() {
    let src = "\
## Component Site
View:
    <main class=\"page\">{block content}</main>
Style:
    .page { padding: 1rem; }

## Component App
View:
    {extends Site}
    {block content:}<p class=\"page\">hi</p>{end}

## Main
Mount App.
";
    let out = rendered(src);
    assert!(
        out.contains(".page.lgx-site { padding: 1rem; }"),
        "the layout's rule: {out}"
    );
    assert!(
        out.contains("<main class=\"page lgx-site\">"),
        "the layout's element is scoped: {out}"
    );
    assert!(
        out.contains("<p class=\"page\">"),
        "the page's element is NOT the layout's: {out}"
    );
}

#[test]
fn an_inspect_in_a_view_renders_the_matching_arm() {
    let src = "\
## Component App
State:
    Let v be some 3.
View:
    {inspect v:}{when Some (n):}<p>got {n}</p>{when None:}<p>nothing</p>{end}

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p>got 3</p>");
}

#[test]
fn an_inspect_falls_through_to_the_arm_that_matches() {
    let src = "\
## Component App
State:
    Let v be none.
View:
    {inspect v:}{when Some (n):}<p>got {n}</p>{when None:}<p>nothing</p>{end}

## Main
Mount App.
";
    assert_eq!(rendered(src), "<p>nothing</p>");
}

/// `source`'s every `## Test` passes on the reference tier and traces identically — document,
/// journal, focus — on the lowered VM, and the page builds for the browser.
fn passes_on_every_tier(source: &str) {
    let report = run_tests(source, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the fixture must parse and run: {e}\n{source}"));
    assert!(
        !report.outcomes.is_empty(),
        "the fixture declares at least one ## Test"
    );
    for outcome in &report.outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`{}` must pass on the reference tier and trace identically on the lowered VM: {:?}",
            outcome.name,
            outcome.failures
        );
    }
    let built = compile_to_wasm(source);
    assert!(
        built.is_ok(),
        "the page must build for the browser: {:?}",
        built.err()
    );
}

#[test]
fn a_layouts_class_toggle_and_listener_reach_the_lowered_page() {
    passes_on_every_tier(
        r##"
## Store Hits

State:
    Let n be 0.

Actions:
    To hit:
        Set n to n + 1.

## Component Site

Use Hits.

Actions:
    To go:
        Call Hits's hit.

View:
    <nav><a id="home" class="k" class:active={true} class:off={false}>home</a><button id="go" on-click={go}>go</button></nav>
    <main>{block content}</main>

## Page /

Use Hits.

View:
    {extends Site}
    {block content:}<p id="count">{Hits's n}</p>{end}

## Test: the toggle composes into the class list and the layout's own action answers its button

Expect that the count of "a.active" is equal to 1.
Expect that the count of "a.off" is equal to 0.
Expect that the count of "a.k" is equal to 1.
Click "#go".
Click "#go".
Expect that the text of "#count" is equal to "2".
"##,
    );
}

#[test]
fn a_styled_layouts_toggle_composes_with_its_scope_class_on_every_tier() {
    passes_on_every_tier(
        r##"
## Component Site

View:
    <main id="m" class="page" class:wide={true}>{block content}</main>

Style:
    .page { padding: 1rem; }

## Page /

View:
    {extends Site}
    {block content:}<p id="p">hi</p>{end}

## Test: the literal class, the toggle and the layout's scope land on one element

Expect that the count of "main.page.wide.lgx-site" is equal to 1.
"##,
    );
}

#[test]
fn a_listener_inside_a_filled_block_answers_on_every_tier() {
    passes_on_every_tier(
        r##"
## Component Site

View:
    <header>nav</header>
    <main>{block content}</main>

## Page /

State:
    Let n be 0.

Actions:
    To add:
        Set n to n + 1.

View:
    {extends Site}
    {block content:}<button id="add" on-click={add}>+</button><p id="n">{n}</p>{end}

## Test: the page's own button, drawn inside the layout's block, counts

Click "#add".
Click "#add".
Expect that the text of "#n" is equal to "2".
"##,
    );
}

#[test]
fn a_layouts_own_action_that_moves_no_store_keeps_the_page_it_is_drawn_in() {
    passes_on_every_tier(
        r##"
## Component Site

Actions:
    To note:
        Let seen be 1.

View:
    <div id="shell"><nav><button id="note" on-click={note}>note</button></nav><main>{block content}</main></div>

## Page /

State:
    Let n be 0.

Actions:
    To add:
        Set n to n + 1.

View:
    {extends Site}
    {block content:}<button id="add" on-click={add}>+</button><p id="n">{n}</p>{end}

## Test: a layout's turn repaints through the page, never into a mount point of its own

Click "#add".
Click "#note".
Click "#add".
Expect that the text of "#n" is equal to "2".
"##,
    );
}

/// A layout with `State:` is an instance of its own, drawn inside the page that extends it: the
/// page's blocks read the page's fields, the layout's markup reads the layout's, and a turn of
/// either keeps what the other holds.
#[test]
fn a_stateful_layout_and_its_page_each_keep_their_own_state_on_every_tier() {
    passes_on_every_tier(
        r##"
## Component Site

State:
    Let open be false.
    Let name be "layout".

Actions:
    To toggle:
        Toggle open.

View:
    <nav><button id="menu" on-click={toggle}>menu</button>{if open:}<ul id="links"><li>home</li></ul>{end}</nav>
    <main>{block content}</main>
    <footer id="f">{name}</footer>

## Page /

State:
    Let name be "page".
    Let n be 0.

Actions:
    To add:
        Set n to n + 1.

View:
    {extends Site}
    {block content:}<p id="who">{name}</p><button id="add" on-click={add}>+</button><p id="n">{n}</p>{end}

## Test: each scope reads its own state, and each turn keeps the other's

Expect that the text of "#who" is equal to "page".
Expect that the text of "#f" is equal to "layout".
Click "#menu".
Expect that the count of "#links" is equal to 1.
Click "#add".
Expect that the count of "#links" is equal to 1.
Expect that the text of "#n" is equal to "1".
Click "#menu".
Expect that the count of "#links" is equal to 0.
Expect that the text of "#n" is equal to "1".
"##,
    );
}

/// A block the page leaves unfilled draws the layout's default, in the LAYOUT's scope.
#[test]
fn an_unfilled_blocks_default_reads_the_layouts_state_on_every_tier() {
    passes_on_every_tier(
        r##"
## Component Site

State:
    Let brand be "Acme".

View:
    <header>{block title:}<h1 id="t">{brand}</h1>{end}</header>
    <main>{block content}</main>

## Page /

State:
    Let brand be "Page".

View:
    {extends Site}
    {block content:}<p id="p">{brand}</p>{end}

## Test: the default is the layout's, the filled block the page's

Expect that the text of "#t" is equal to "Acme".
Expect that the text of "#p" is equal to "Page".
"##,
    );
}

#[test]
fn a_bound_input_inside_a_filled_block_writes_back_on_every_tier() {
    passes_on_every_tier(
        r##"
## Component Site

View:
    <header>nav</header>
    <main>{block content}</main>

## Page /

State:
    Let note be "".
    Let sent be false.

Actions:
    To send:
        Set sent to true.

View:
    {extends Site}
    {block content:}
      {if sent:}
        <p id="thanks">sent {note}</p>
      {else:}
        <form id="say" on-submit.prevent={send}>
          <input id="note" type="text" aria-label="note" bind={note}>
          <button type="submit">Send</button>
        </form>
      {end}
    {end}

## Test: the block's bound input is written back and the form's action reads it

Type "hello" into "#note".
Submit "#say".
Expect that the text of "#thanks" is equal to "sent hello".
"##,
    );
}

#[test]
fn a_bound_input_handed_to_a_placement_as_children_writes_back_on_every_tier() {
    passes_on_every_tier(
        r##"
## Component Card

View:
    <section class="card">{children}</section>

## Page /

State:
    Let name be "".

View:
    <Card><input id="name" type="text" aria-label="name" bind={name}></Card>
    <p id="shown">{name}</p>

## Test: the page's own field, bound inside a card's children, moves

Type "ada" into "#name".
Expect that the text of "#shown" is equal to "ada".
"##,
    );
}

/// ★ ONE ELEMENT EMITTER. Every attribute kind the view IR has (`ast::view::Attr`, read from its
/// source) is drawn by `emit_element_attrs`, and no other function in the render walk iterates an
/// element's attributes — a second walk is where the layout path silently dropped every attribute
/// that was not `Plain`.
#[test]
fn every_attribute_kind_is_drawn_by_the_one_element_emitter() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_language/src");
    let view = std::fs::read_to_string(root.join("ast/view.rs")).expect("ast/view.rs");
    let render =
        std::fs::read_to_string(root.join("parser/lgx_lower/render.rs")).expect("render.rs");
    let enum_body = view
        .split("pub enum Attr<'a> {")
        .nth(1)
        .and_then(|rest| rest.split("\n}").next())
        .expect("ast/view.rs declares `pub enum Attr<'a>`");
    let variants: Vec<&str> = enum_body
        .lines()
        .map(str::trim)
        .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
        .filter_map(|l| l.split(|c: char| !c.is_alphanumeric()).next())
        .collect();
    assert!(
        variants.len() >= 4,
        "read the Attr variants from source: {variants:?}"
    );
    let emitter = render
        .split("fn emit_element_attrs(")
        .nth(1)
        .and_then(|rest| rest.split("\n    pub(crate) fn ").next())
        .expect("render.rs defines `fn emit_element_attrs`");
    for v in &variants {
        assert!(
            emitter.contains(&format!("Attr::{v}")),
            "`Attr::{v}` is drawn by the one element emitter"
        );
    }
    let walks = render.matches("for attr in attrs").count();
    let inside = emitter.matches("for attr in attrs").count();
    assert_eq!(
        walks, inside,
        "only `emit_element_attrs` walks an element's attributes in render.rs"
    );
    assert!(
        !render.contains("let Attr::"),
        "no attribute kind is skipped by a `let … else` in render.rs"
    );
}
