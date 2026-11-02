//! ★ LOCK — an LGX page renders IDENTICALLY on the tree-walker and the bytecode VM. ★
//!
//! The tree-walker is the reference semantics for the component model; the VM is the tier a
//! browser actually runs, because a page reaches the browser as VM bytecode lowered to
//! WebAssembly. Two independent implementations of the same rendering rules are only safe if
//! something holds them together, and this is that thing: the same source, the same page, byte
//! for byte.
//!
//! It grows one construct at a time. Anything not yet compiled by the VM is refused LOUDLY
//! rather than rendered wrongly, and `the_vm_refuses_what_it_cannot_yet_render` pins that the
//! refusal stays loud — a silent no-op would mean a page that works on one tier and is blank on
//! the other with nothing said.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// The page `src` renders on the tree-walker.
fn tw_page(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "the tree-walker must run it cleanly:\n{src}"
    );
    dom::render()
}

/// The page `src` renders on the bytecode VM.
fn vm_page(src: &str) -> String {
    dom::reset();
    let out = vm_outcome(src);
    assert_eq!(out.error, None, "the VM must run it cleanly:\n{src}");
    dom::render()
}

/// Both tiers, held together.
fn agree(name: &str, src: &str) -> String {
    let tw = tw_page(src);
    let vm = vm_page(src);
    assert_eq!(
        tw, vm,
        "`{name}` renders differently on the two tiers:\n{src}"
    );
    tw
}

#[test]
fn an_element_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "element",
            "## Component App\nView:\n    <div></div>\n\n## Main\nMount App.\n"
        ),
        "<body><div></div></body>"
    );
}

#[test]
fn text_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "text",
            "## Component App\nView:\n    <h1>Hello</h1>\n\n## Main\nMount App.\n"
        ),
        "<body><h1>Hello</h1></body>"
    );
}

#[test]
fn attributes_render_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "attributes",
            "## Component App\nView:\n    <a href=\"/x\" id=\"go\">link</a>\n\n## Main\nMount App.\n"
        ),
        "<body><a href=\"/x\" id=\"go\">link</a></body>"
    );
}

#[test]
fn nested_elements_render_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "nesting",
            "## Component App\nView:\n    <ul><li><b>a</b></li><li>b</li></ul>\n\n## Main\nMount App.\n"
        ),
        "<body><ul><li><b>a</b></li><li>b</li></ul></body>"
    );
}

#[test]
fn a_state_hole_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "hole",
            "## Component App\nState:\n    Let n be 7.\nView:\n    <p>Count: {n}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>Count: 7</p></body>"
    );
}

#[test]
fn a_computed_hole_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "computed hole",
            "## Component App\nState:\n    Let n be 3.\nView:\n    <p>{n * 2}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>6</p></body>"
    );
}

#[test]
fn a_text_hole_is_escaped_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "escaping",
            "## Component App\nState:\n    Let t be \"<b>\".\nView:\n    <p>{t}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>&lt;b&gt;</p></body>"
    );
}

#[test]
fn a_dynamic_attribute_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "dynamic attribute",
            "## Component App\nState:\n    Let who be \"ada\".\nView:\n    <p title={who}>x</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p title=\"ada\">x</p></body>"
    );
}

#[test]
fn a_derived_field_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "derived",
            "## Component App\nState:\n    Let n be 4.\n    Let doubled always be n * 2.\nView:\n    <p>{doubled}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>8</p></body>"
    );
}

#[test]
fn mounting_at_a_selector_agrees_on_both_tiers() {
    assert_eq!(
        agree(
            "mount at",
            "## Component Shell\nView:\n    <div id=\"root\"></div>\n\n## Component App\nView:\n    <h1>hi</h1>\n\n## Main\nMount Shell.\nMount App at \"#root\".\n"
        ),
        "<body><div id=\"root\"><h1>hi</h1></div></body>"
    );
}

#[test]
fn a_conditional_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "if",
            "## Component App\nState:\n    Let ready be true.\nView:\n    {if ready:}<p>yes</p>{end}\n\n## Main\nMount App.\n"
        ),
        "<body><p>yes</p></body>"
    );
    assert_eq!(
        agree(
            "if / else",
            "## Component App\nState:\n    Let ready be false.\nView:\n    {if ready:}<p>yes</p>{else:}<p>no</p>{end}\n\n## Main\nMount App.\n"
        ),
        "<body><p>no</p></body>"
    );
}

#[test]
fn an_else_if_chain_takes_the_same_arm_on_both_tiers() {
    assert_eq!(
        agree(
            "else if",
            "## Component App\nState:\n    Let n be 2.\nView:\n    {if n is equal to 1:}<p>one</p>{else if n is equal to 2:}<p>two</p>{else:}<p>many</p>{end}\n\n## Main\nMount App.\n"
        ),
        "<body><p>two</p></body>"
    );
}

#[test]
fn a_loop_renders_the_same_rows_on_both_tiers() {
    assert_eq!(
        agree(
            "for each",
            "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs:}<li>{x}</li>{end}</ul>\n\n## Main\nMount App.\n"
        ),
        "<body><ul><li>a</li><li>b</li></ul></body>"
    );
}

#[test]
fn an_empty_loop_takes_its_else_arm_on_both_tiers() {
    assert_eq!(
        agree(
            "for each / else",
            "## Component App\nState:\n    Let xs be a new Seq of Text.\nView:\n    <ul>{for each x in xs:}<li>{x}</li>{else:}<li>none</li>{end}</ul>\n\n## Main\nMount App.\n"
        ),
        "<body><ul><li>none</li></ul></body>"
    );
}

#[test]
fn a_loop_index_counts_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "at index",
            "## Component App\nState:\n    Let xs be [\"a\", \"b\", \"c\"].\nView:\n    <ul>{for each x in xs, at index i:}<li>{i}:{x}</li>{end}</ul>\n\n## Main\nMount App.\n"
        ),
        "<body><ul><li>1:a</li><li>2:b</li><li>3:c</li></ul></body>"
    );
}

#[test]
fn a_template_binding_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "let",
            "## Component App\nState:\n    Let n be 3.\nView:\n    {let doubled be n * 2}<p>{doubled}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>6</p></body>"
    );
}

#[test]
fn a_loop_of_elements_with_attributes_agrees_on_both_tiers() {
    assert_eq!(
        agree(
            "loop with attributes",
            "## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs:}<li class=\"row\" title={x}>{x}</li>{end}</ul>\n\n## Main\nMount App.\n"
        ),
        "<body><ul><li class=\"row\" title=\"a\">a</li><li class=\"row\" title=\"b\">b</li></ul></body>"
    );
}

#[test]
fn a_child_component_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "child",
            "## Component Card\nView:\n    <p>card</p>\n\n## Component App\nView:\n    <div><Card/></div>\n\n## Main\nMount App.\n"
        ),
        "<body><div><p>card</p></div></body>"
    );
}

#[test]
fn a_prop_reaches_the_child_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "prop",
            "## Component Greeting (who: Text)\nView:\n    <p>Hi {who}</p>\n\n## Component App\nState:\n    Let name be \"ada\".\nView:\n    <Greeting who={name}/>\n\n## Main\nMount App.\n"
        ),
        "<body><p>Hi ada</p></body>"
    );
}

#[test]
fn props_are_matched_by_name_not_position_on_both_tiers() {
    assert_eq!(
        agree(
            "props by name",
            "## Component Pair (first: Text) and (second: Text)\nView:\n    <p>{first}-{second}</p>\n\n## Component App\nView:\n    <Pair second={\"b\"} first={\"a\"}/>\n\n## Main\nMount App.\n"
        ),
        "<body><p>a-b</p></body>"
    );
}

#[test]
fn a_child_in_a_loop_renders_once_per_item_on_both_tiers() {
    assert_eq!(
        agree(
            "child in a loop",
            "## Component Row (label: Text)\nView:\n    <li>{label}</li>\n\n## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs:}<Row label={x}/>{end}</ul>\n\n## Main\nMount App.\n"
        ),
        "<body><ul><li>a</li><li>b</li></ul></body>"
    );
}

#[test]
fn a_child_declared_below_its_parent_still_renders_on_both_tiers() {
    assert_eq!(
        agree(
            "forward reference",
            "## Component App\nView:\n    <div><Card/></div>\n\n## Component Card\nView:\n    <p>card</p>\n\n## Main\nMount App.\n"
        ),
        "<body><div><p>card</p></div></body>"
    );
}

#[test]
fn a_parent_of_an_unlowerable_child_is_refused_rather_than_rendered_short() {
    // The page would otherwise come out missing a piece with nothing said, which is the exact
    // failure the loud refusal exists to prevent.
    // ★ THIS FIXTURE HAS NOW MOVED THREE TIMES, AND EACH MOVE WAS THE SAME LESSON.
    //
    // It was `When the window resizes:`, "chosen because it stays unlowerable as the refusal list
    // shrinks" — then the window events landed. It became `When a message arrives on …:`, "the one
    // trigger deliberately out of scope" — then the relay turned out to be built and that landed
    // too. Every trigger the language declares now reaches a browser, so no TRIGGER can be this
    // test's subject ever again.
    //
    // ★ SO THE SUBJECT IS UNDECIDABLE BY NATURE, NOT A GAP. A `bind` on an input whose `type` is
    // computed cannot be told, before the page runs, whether it writes `value` or a checkbox's
    // `checked` — so the lowering refuses it by name rather than guess, and no amount of further
    // work changes that. The child is unlowerable for that reason; the parent must not be drawn
    // around a missing piece.
    let src = "## Component Row\nState:\n    Let draft be \"\".\n    Let kind be \"text\".\nView:\n    <li><input aria-label=\"d\" type={kind} bind={draft}></li>\n\n## Component App\nView:\n    <ul><Row/></ul>\n\n## Main\nMount App.\n";
    dom::reset();
    let error = vm_outcome(src)
        .error
        .expect("the parent must be refused too");
    assert!(
        error.contains("App") && error.contains("tier"),
        "names the mount it refused: {error}"
    );
}

#[test]
fn a_class_toggle_composes_the_same_list_on_both_tiers() {
    assert_eq!(
        agree(
            "class toggle",
            "## Component App\nState:\n    Let done be true.\nView:\n    <li class=\"row\" class:done={done}>x</li>\n\n## Main\nMount App.\n"
        ),
        "<body><li class=\"row done\">x</li></body>"
    );
}

#[test]
fn a_false_class_toggle_is_absent_on_both_tiers() {
    assert_eq!(
        agree(
            "class toggle off",
            "## Component App\nState:\n    Let done be false.\nView:\n    <li class=\"row\" class:done={done}>x</li>\n\n## Main\nMount App.\n"
        ),
        "<body><li class=\"row\">x</li></body>"
    );
}

#[test]
fn two_class_toggles_both_land_on_both_tiers() {
    assert_eq!(
        agree(
            "two toggles",
            "## Component App\nState:\n    Let a be true.\n    Let b be true.\nView:\n    <li class:one={a} class:two={b}>x</li>\n\n## Main\nMount App.\n"
        ),
        "<body><li class=\"one two\">x</li></body>"
    );
}

#[test]
fn an_element_with_no_class_at_all_stays_bare_on_both_tiers() {
    assert_eq!(
        agree(
            "no class",
            "## Component App\nState:\n    Let off be false.\nView:\n    <li class:done={off}>x</li>\n\n## Main\nMount App.\n"
        ),
        "<body><li>x</li></body>"
    );
}

/// ★ `Save … as` / `Load … into` ROUND-TRIP A TYPED VALUE, identically on both tiers.
///
/// These were refused on the browser tier for a real reason, not missing work: the reference tier
/// stores a typed `RuntimeValue`, a browser's storage holds only Text, and mapping `Save n as "k"`
/// straight onto Text would hand `Load` a Text where the tree-walker hands an Int — a silent
/// divergence in a value the page then computes with.
///
/// Closed by making the stored text SELF-DESCRIBING (`i:42`), with the tag written and read in
/// compiled LOGOS over the storage imports that were already frozen. This test is the proof that
/// matters: it loads into an Int field and then does ARITHMETIC on it, so a Text sneaking through
/// would not merely look different — it would fail to compute.
#[test]
fn a_saved_value_loads_back_as_its_own_type_on_both_tiers() {
    // ★ EVERY TYPE THE TAG CAN NAME. Browser storage holds only TEXT, so `Load` must be told what
    // came out — and the first lowering asked the wrong question, reading only a field's WRITTEN
    // annotation. `Let seen be 0.` carries none, so every ordinary field decoded as Text: a saved
    // 7 came back as "7" and `seen + 1` CONCATENATED to "71" instead of adding to 8. A wrong
    // answer, not a refusal. One row per type, each doing something only the right type can do, so
    // a decode that quietly falls back to Text cannot pass.
    for (label, saved, start, step, want) in [
        ("Int", "7", "0", "Set seen to seen + 1.", "8"),
        ("Text", "\"hi\"", "\"\"", "Set seen to seen + \"!\".", "hi!"),
        ("Float", "1.5", "0.0", "Set seen to seen + 1.0.", "2.5"),
    ] {
        let src = format!(
            "## Component App\nState:\n    Let n be {saved}.\n    Let seen be {start}.\nActions:\n\
             \x20   To stash:\n        Save n as \"k\".\n    To fetch:\n        Load \"k\" into seen.\n\
             \x20       {step}\nView:\n    <p id=\"s\">{{seen}}</p>\
             <button id=\"a\" on-click={{stash}}>a</button>\
             <button id=\"b\" on-click={{fetch}}>b</button>\n\n\
             ## Main\nMount App.\nClick \"#a\".\nClick \"#b\".\n"
        );
        let got = agree(&format!("save/load {label}"), &src);
        assert!(
            got.contains(&format!(">{want}<")),
            "a saved {label} must come back as a {label} on BOTH tiers — wanted >{want}< in {got}"
        );
    }

    // A Bool cannot be added to, so it proves itself by being asked a question only a Bool answers.
    assert_eq!(
        agree(
            "save/load Bool",
            "## Component App\nState:\n    Let flag be true.\n    Let seen be false.\nActions:\n    \
             To stash:\n        Save flag as \"k\".\n    To fetch:\n        Load \"k\" into seen.\n\
             View:\n    <p id=\"s\">{if seen:}yes{else:}no{end}</p>\
             <button id=\"a\" on-click={stash}>a</button><button id=\"b\" on-click={fetch}>b</button>\n\n\
             ## Main\nMount App.\nClick \"#a\".\nClick \"#b\".\n"
        ),
        "<body><p id=\"s\">yes</p><button id=\"a\">a</button><button id=\"b\">b</button></body>",
        "a saved Bool must come back as a Bool — a Text decode cannot be asked `{{if seen:}}`"
    );
}

/// ★ `Forget` MUST ACTUALLY FORGET, and a `Load` of a forgotten key must not invent a value.
///
/// The pair is only honest together: a `Save` that cannot be undone leaves a page unable to log
/// anyone out, and a `Load` that answers a stale value after a `Forget` is the same wrong-answer
/// shape the type tag was written to close.
#[test]
fn a_forgotten_key_loads_as_the_empty_value_on_both_tiers() {
    assert_eq!(
        agree(
            "save then forget",
            "## Component App\nState:\n    Let n be 7.\n    Let seen be 0.\nActions:\n    \
             To stash:\n        Save n as \"k\".\n    To drop:\n        Forget \"k\".\n    \
             To fetch:\n        Load \"k\" into seen.\nView:\n    <p id=\"s\">{seen}</p>\
             <button id=\"a\" on-click={stash}>a</button><button id=\"d\" on-click={drop}>d</button>\
             <button id=\"b\" on-click={fetch}>b</button>\n\n\
             ## Main\nMount App.\nClick \"#a\".\nClick \"#d\".\nClick \"#b\".\n"
        ),
        "<body><p id=\"s\">0</p><button id=\"a\">a</button><button id=\"d\">d</button>\
         <button id=\"b\">b</button></body>",
        "after a Forget, a Load must read the absent value — not the value that was there before"
    );
}

/// ★ `When the user presses "Escape":` LISTENS ON THE PAGE, and answers on both tiers.
///
/// It differs from a keyed SELECTOR trigger only in which node carries the listener, and it needed
/// no new host import: `body`, `addEventListener` and `eventKey` were already frozen in the ABI and
/// `listener_closure` already threaded a key filter. Only the wiring was missing — the same shape
/// `When the route changes:` was in.
///
/// ★ AND THE NODE MUST BE RESOLVED THE WAY THE DRIVER RESOLVES IT. The first lowering registered
/// the listener on the `body()` BUILTIN while the press driver looks its target up with
/// `querySelector` — two handles for one node, so the page compiled, mounted, registered, and
/// answered "Nothing answers a press on \"body\"". The listener was there and unreachable. Both
/// halves now go through `querySelector`, which is why this test drives the real thing rather than
/// asserting a refusal.
#[test]
fn a_page_level_key_press_takes_the_same_arm_on_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    When the user presses \"Escape\":\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n\n## Main\nMount App.\nPress \"Escape\".\n";
    assert_eq!(
        agree("page key press", SRC),
        "<body><p id=\"n\">1</p></body>"
    );
}

#[test]
fn an_inspect_takes_the_same_arm_on_both_tiers() {
    assert_eq!(
        agree(
            "inspect some",
            "## Component App\nState:\n    Let v be some 3.\nView:\n    {inspect v:}{when Some (n):}<p>got {n}</p>{when None:}<p>-</p>{end}\n\n## Main\nMount App.\n"
        ),
        "<body><p>got 3</p></body>"
    );
    assert_eq!(
        agree(
            "inspect none",
            "## Component App\nState:\n    Let v be none.\nView:\n    {inspect v:}{when Some (n):}<p>got {n}</p>{when None:}<p>-</p>{end}\n\n## Main\nMount App.\n"
        ),
        "<body><p>-</p></body>"
    );
}

#[test]
fn a_click_moves_the_same_state_on_both_tiers() {
    assert_eq!(
        agree(
            "on-click",
            "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n    <button id=\"b\" on-click={bump}>+</button>\n\n## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><p id=\"n\">1</p><button id=\"b\">+</button></body>"
    );
}

#[test]
fn a_when_sentence_moves_the_same_state_on_both_tiers() {
    assert_eq!(
        agree(
            "When clicked",
            "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#b\" is clicked:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n    <button id=\"b\">+</button>\n\n## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><p id=\"n\">1</p><button id=\"b\">+</button></body>"
    );
}

#[test]
fn repeated_turns_accumulate_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "three clicks",
            "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n    <button id=\"b\" on-click={bump}>+</button>\n\n## Main\nMount App.\nClick \"#b\".\nClick \"#b\".\nClick \"#b\".\n"
        ),
        "<body><p id=\"n\">3</p><button id=\"b\">+</button></body>"
    );
}

#[test]
fn an_action_with_an_argument_agrees_on_both_tiers() {
    assert_eq!(
        agree(
            "action argument",
            "## Component App\nState:\n    Let picked be \"\".\nActions:\n    To pick (what: Text):\n        Set picked to what.\nView:\n    <p id=\"p\">{picked}</p>\n    <button id=\"b\" on-click={pick(\"a\")}>a</button>\n\n## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><p id=\"p\">a</p><button id=\"b\">a</button></body>"
    );
}

#[test]
fn a_turn_that_changes_a_list_repaints_the_same_rows_on_both_tiers() {
    assert_eq!(
        agree(
            "list turn",
            "## Component App\nState:\n    Let items be [\"a\"].\nActions:\n    To add:\n        Push \"b\" to items.\nView:\n    <ul>{for each x in items:}<li>{x}</li>{end}</ul>\n    <button id=\"b\" on-click={add}>+</button>\n\n## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><ul><li>a</li><li>b</li></ul><button id=\"b\">+</button></body>"
    );
}

#[test]
fn a_derived_field_recomputes_the_same_after_a_turn_on_both_tiers() {
    assert_eq!(
        agree(
            "derived after a turn",
            "## Component App\nState:\n    Let n be 1.\n    Let doubled always be n * 2.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{doubled}</p>\n    <button id=\"b\" on-click={bump}>+</button>\n\n## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><p id=\"n\">4</p><button id=\"b\">+</button></body>"
    );
}

#[test]
fn a_conditional_view_flips_the_same_after_a_turn_on_both_tiers() {
    assert_eq!(
        agree(
            "conditional turn",
            "## Component App\nState:\n    Let open be false.\nActions:\n    To toggle:\n        Set open to true.\nView:\n    {if open:}<p>open</p>{else:}<p>shut</p>{end}\n    <button id=\"b\" on-click={toggle}>t</button>\n\n## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><p>open</p><button id=\"b\">t</button></body>"
    );
}

#[test]
fn a_component_placed_twice_with_changing_state_renders_both_instances() {
    // ★ MIGRATED, AND THE ASSERTION IS STRICTLY STRONGER. This used to demand a REFUSAL, and it was
    // right to: state lived in module-level bindings, so two placements would have shared one
    // record — the isolation failure the component model exists to prevent, and rendering it would
    // have been worse than refusing it.
    //
    // State is now a map from INSTANCE KEY to value and a placement passes the key of the instance
    // it is drawing, so the two rows keep two records. The test that pinned the gap now pins the
    // capability: both tiers render the page, they render it IDENTICALLY, and each instance starts
    // from its own initializer rather than sharing one.
    let src = "## Component Counter\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <li on-click={bump}>{n}</li>\n\n## Component App\nView:\n    <ul><Counter/><Counter/></ul>\n\n## Main\nMount App.\n";
    assert_eq!(
        agree("two instances", src),
        "<body><ul><li>0</li><li>0</li></ul></body>",
        "both instances render, each from its own state"
    );
}

#[test]
fn a_style_block_scopes_the_same_way_on_both_tiers() {
    assert_eq!(
        agree(
            "style",
            "## Component App\nView:\n    <p class=\"card\">hi</p>\nStyle:\n    .card { color: red }\n\n## Main\nMount App.\n"
        ),
        "<body><style>.card.lgx-app { color: red }</style><p class=\"card lgx-app\">hi</p></body>"
    );
}

#[test]
fn a_styled_child_keeps_its_own_scope_on_both_tiers() {
    assert_eq!(
        agree(
            "child scope",
            "## Component Row\nView:\n    <li class=\"row\">r</li>\nStyle:\n    .row { color: blue }\n\n## Component App\nView:\n    <ul class=\"list\"><Row/></ul>\n\n## Main\nMount App.\n"
        ),
        // A child's stylesheet lands where the child does — inside its parent's element, not
        // hoisted to the page. Both tiers agree, and this is the shape they agree on.
        // ★ THE SHEET MOVED OUT OF THE `<ul>`, and the scope assertions are untouched. A `<ul>` may hold
        // only `<li>` and script-supporting elements, so a `<style>` there is invalid markup a real
        // parser relocates; every tier now emits sheets at the mount point ahead of the markup. What
        // this test is ABOUT — that a styled child carries its own `lgx-row`, not its parent's — is
        // exactly as before.
        "<body><style>.row.lgx-row { color: blue }</style><ul class=\"list\"><li class=\"row lgx-row\">r</li></ul></body>"
    );
}

#[test]
fn a_bound_input_shows_its_field_on_both_tiers() {
    assert_eq!(
        agree(
            "bind shows",
            "## Component App\nState:\n    Let draft be \"milk\".\nView:\n    <input id=\"d\" bind={draft}>\n\n## Main\nMount App.\n"
        ),
        // Updated when the serializer stopped writing a closing tag for a VOID element: an
        // `</input>` is a stray tag HTML's own parser drops, and this line was pinning it. The
        // claim — the two tiers render this identically — is unchanged. See
        // `void_element_serialize_lock`.
        "<body><input id=\"d\" value=\"milk\"></body>"
    );
}

#[test]
fn typing_into_a_bound_input_writes_the_field_on_both_tiers() {
    assert_eq!(
        agree(
            "bind writes",
            "## Component App\nState:\n    Let draft be \"\".\nView:\n    <input id=\"d\" bind={draft}>\n    <p id=\"e\">{draft}</p>\n\n## Main\nMount App.\nType \"milk\" into \"#d\".\n"
        ),
        // Updated when the serializer stopped writing a closing tag for a VOID element: an
        // `</input>` is a stray tag HTML's own parser drops, and this line was pinning it. The
        // claim — the two tiers render this identically — is unchanged. See
        // `void_element_serialize_lock`.
        "<body><input id=\"d\" value=\"milk\"><p id=\"e\">milk</p></body>"
    );
}

#[test]
fn a_changes_sentence_answers_typing_on_both_tiers() {
    // Typing raises an `input` and then a `change`, so a `changes` sentence answers a keystroke
    // on this tier for the same reason it does on the reference one.
    assert_eq!(
        agree(
            "changes answers typing",
            "## Component App\nState:\n    Let n be 0.\nActions:\n    When \"#d\" changes:\n        Set n to n + 1.\nView:\n    <input id=\"d\">\n    <p id=\"e\">{n}</p>\n\n## Main\nMount App.\nType \"x\" into \"#d\".\n"
        ),
        // Typing's default action writes the control's value, and a control whose value differs
        // from its markup serializes it as `value="…"`, as a browser's document does after typing.
        "<body><input id=\"d\" value=\"x\"><p id=\"e\">1</p></body>"
    );
}

#[test]
fn a_selector_the_page_does_not_answer_fails_the_same_way_on_both_tiers() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <button id=\"b\" on-click={bump}>+</button>\n\n## Main\nMount App.\nClick \"#missing\".\n";
    dom::reset();
    let tw = tw_outcome(src)
        .error
        .expect("the tree-walker must refuse it");
    dom::reset();
    let vm = vm_outcome(src).error.expect("the VM must refuse it too");
    // Not merely similar: the same message. A page that fails differently depending on the tier
    // that ran it is a page whose tests read differently depending on the tier that ran them.
    assert_eq!(tw, vm, "the two tiers must fail with the SAME words");
    assert!(tw.contains("#missing"), "and must name the selector: {tw}");
}

#[test]
fn a_directive_runs_against_its_element_on_both_tiers() {
    assert_eq!(
        agree(
            "directive",
            "## Directive tip (el: Element) and (t: Text):\n    Call setAttribute with el and \"title\" and t.\n\n## Component App\nView:\n    <p tip={\"hint\"}>y</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p title=\"hint\">y</p></body>"
    );
}

#[test]
fn a_directive_reads_state_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "directive over state",
            "## Directive tip (el: Element) and (t: Text):\n    Call setAttribute with el and \"title\" and t.\n\n## Component App\nState:\n    Let hint be \"from state\".\nView:\n    <p tip={hint}>y</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p title=\"from state\">y</p></body>"
    );
}

#[test]
fn a_directive_with_a_removed_block_runs_both_halves_rather_than_half_run() {
    // Never half-run: the mount half runs on the element the paint draws, and the cleanup half runs
    // when a repaint no longer draws it — on both tiers, with the value it was attached with.
    let src = "## Directive watched (el: Element) and (t: Text):\n    Call setAttribute with el and \"title\" and t.\n    When removed:\n        Show \"gone \" + t.\n\n## Component App\nState:\n    Let on be true.\nActions:\n    To hide:\n        Set on to false.\nView:\n    <button id=\"h\" on-click={hide}>h</button>{if on:}<p watched={\"x\"}>y</p>{end}\n\n## Main\nMount App.\n";
    assert_eq!(
        agree("directive mount half", src),
        "<body><button id=\"h\">h</button><p title=\"x\">y</p></body>"
    );
    let hidden = format!("{src}Click \"#h\".\n");
    assert_eq!(
        agree("directive cleanup half", &hidden),
        "<body><button id=\"h\">h</button></body>"
    );
    dom::reset();
    let tw = tw_outcome(&hidden).output;
    dom::reset();
    let vm = vm_outcome(&hidden).output;
    assert_eq!(tw, vm, "both tiers run the cleanup half the same");
    assert!(
        vm.contains("gone x"),
        "the cleanup half ran with the value it was attached with: {vm:?}"
    );
}

#[test]
fn a_store_field_reads_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "store read",
            "## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n\n## Component App\nUse Session.\nView:\n    <p>{Session's name}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>guest</p></body>"
    );
}

#[test]
fn calling_a_store_action_changes_what_the_page_shows_on_both_tiers() {
    assert_eq!(
        agree(
            "store write",
            "## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n\n## Component App\nUse Session.\nActions:\n    To signIn:\n        Call Session's login with \"ada\".\nView:\n    <p id=\"n\">{Session's name}</p>\n    <button id=\"b\" on-click={signIn}>in</button>\n\n## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><p id=\"n\">ada</p><button id=\"b\">in</button></body>"
    );
}

#[test]
fn a_store_action_with_no_arguments_agrees_on_both_tiers() {
    assert_eq!(
        agree(
            "store no args",
            "## Store Counter\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\n\n## Component App\nUse Counter.\nActions:\n    To go:\n        Call Counter's bump.\nView:\n    <p id=\"n\">{Counter's n}</p>\n    <button id=\"b\" on-click={go}>+</button>\n\n## Main\nMount App.\nClick \"#b\".\nClick \"#b\".\n"
        ),
        "<body><p id=\"n\">2</p><button id=\"b\">+</button></body>"
    );
}

#[test]
fn a_store_shared_by_two_components_shows_the_same_thing_on_both_tiers() {
    assert_eq!(
        agree(
            "shared store",
            "## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.\n\n## Component Greeting\nUse Session.\nView:\n    <em>{Session's name}</em>\n\n## Component App\nUse Session.\nActions:\n    To signIn:\n        Call Session's login with \"ada\".\nView:\n    <p>{Session's name}</p>\n    <Greeting/>\n    <button id=\"b\" on-click={signIn}>in</button>\n\n## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><p>ada</p><em>ada</em><button id=\"b\">in</button></body>"
    );
}

#[test]
fn a_store_seq_field_is_typed_from_its_declaration_on_every_tier() {
    // A record needs a type per field, and a guessed one miscompiles the page that depends on it.
    // A store's `Seq of Text` field is not guessed: its declaration names the element type, so the
    // lowered store is a real record and every pushed element comes back as the Text it was — its
    // length AND its elements render the same on both tiers.
    let src = "## Store S\nState:\n    Let items be a new Seq of Text.\nActions:\n    To add:\n        Push \"a\" to items.\n\n## Component App\nUse S.\nActions:\n    To go:\n        Call S's add.\nView:\n    <p id=\"n\">{length of S's items}</p>\n    <ul>{for each i in S's items:}<li>{i}</li>{end}</ul>\n    <button id=\"b\" on-click={go}>+</button>\n\n## Main\nMount App.\nClick \"#b\".\nClick \"#b\".\n";
    assert_eq!(
        agree("store seq field", src),
        "<body><p id=\"n\">2</p><ul><li>a</li><li>a</li></ul><button id=\"b\">+</button></body>"
    );
}

#[test]
fn a_page_loads_handler_runs_before_the_first_paint_on_both_tiers() {
    // What it sets is what the page first shows — on both tiers, or the sentence is decorative.
    assert_eq!(
        agree(
            "page loads",
            "## Component App\nState:\n    Let n be 0.\nActions:\n    When the page loads:\n        Set n to 41.\nView:\n    <p id=\"n\">{n}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p id=\"n\">41</p></body>"
    );
}

#[test]
fn an_appears_handler_runs_before_the_first_paint_on_both_tiers() {
    assert_eq!(
        agree(
            "appears",
            "## Component App\nState:\n    Let n be 0.\nActions:\n    When this component appears:\n        Set n to 7.\nView:\n    <p id=\"n\">{n}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p id=\"n\">7</p></body>"
    );
}

#[test]
fn a_load_handler_runs_once_and_not_again_on_a_repaint_on_both_tiers() {
    // The render function runs on every repaint; a load handler must not. If it did, every click
    // would re-run the page's setup, which is the bug this separation exists to prevent.
    assert_eq!(
        agree(
            "load runs once",
            "## Component App\nState:\n    Let loads be 0.\n    Let clicks be 0.\nActions:\n    When the page loads:\n        Set loads to loads + 1.\n    To bump:\n        Set clicks to clicks + 1.\nView:\n    <p id=\"n\">{loads}-{clicks}</p>\n    <button id=\"b\" on-click={bump}>+</button>\n\n## Main\nMount App.\nClick \"#b\".\nClick \"#b\".\n"
        ),
        "<body><p id=\"n\">1-2</p><button id=\"b\">+</button></body>"
    );
}

/// ★ The honesty half. A construct the VM cannot yet render is REFUSED, loudly, naming what it
/// cannot do — never rendered partially and never skipped in silence. As constructs land they
// ---- Edges. The tests above cover the shapes a page is USUALLY made of; these cover the ones
// where two independent implementations drift apart quietly — empty values, hostile characters,
// numbers that are not integers, and the boundaries of a loop. Each is a place where "the tree
// looked right" is easy to believe and wrong.

#[test]
fn a_hole_in_an_attribute_value_is_escaped_the_same_on_both_tiers() {
    // The text-hole escaping test above covers element CONTENT. An attribute is the harder half:
    // a bare `"` there ends the value and everything after it becomes markup the author did not
    // write. Both tiers must escape it, and identically.
    assert_eq!(
        agree(
            "attribute escaping",
            "## Component App\nState:\n    Let t be \"a\\\"b<c&d\".\nView:\n    <p title={t}>x</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p title=\"a&quot;b&lt;c&amp;d\">x</p></body>"
    );
}

#[test]
fn an_empty_hole_and_an_empty_attribute_agree_on_both_tiers() {
    // Empty is not nothing: an empty attribute must still be PRESENT (`alt=""` is meaningful to a
    // screen reader), while an empty class list must leave the element bare rather than carry a
    // stray `class=""`. Two opposite rules that are easy to get backwards on one tier only.
    assert_eq!(
        agree(
            "empty values",
            "## Component App\nState:\n    Let e be \"\".\nView:\n    <img src=\"/a.png\" alt={e}>\n\n## Main\nMount App.\n"
        ),
        // Attributes serialize ALPHABETICALLY (the recorder holds them in a `BTreeMap`), not in
        // source order — `alt` before `src` regardless of how the template is written. The op-log
        // twins pin the source ORDER of the calls; this pins the rendered tree.
        // Updated when the serializer stopped closing VOID elements — see the note above and
        // `void_element_serialize_lock`. The claim (both tiers agree) is unchanged.
        "<body><img alt=\"\" src=\"/a.png\"></body>"
    );
}

#[test]
fn a_multibyte_hole_renders_the_same_on_both_tiers() {
    // Text crosses the tiers as a HANDLE, and the byte length of what is behind it is not its
    // character count. A hole holding non-ASCII is where a length computed in the wrong unit
    // shows up as a truncated or mangled page.
    assert_eq!(
        agree(
            "multibyte",
            "## Component App\nState:\n    Let t be \"héllo — 世界 🌍\".\nView:\n    <p>{t}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>héllo — 世界 🌍</p></body>"
    );
}

#[test]
fn a_negative_and_a_fractional_hole_render_the_same_on_both_tiers() {
    // A hole is coerced to text before it reaches a DOM call, and that coercion is exactly where
    // an earlier miscompile lived: an Int handed to an import expecting a Text handle produced a
    // module that compiled and then failed to VALIDATE. A negative and a Real exercise the two
    // number paths through it.
    assert_eq!(
        agree(
            "negative and real holes",
            "## Component App\nState:\n    Let n be 0 - 42.\n    Let r be 1.5.\nView:\n    <p>{n} and {r}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>-42 and 1.5</p></body>"
    );
}

#[test]
fn a_hole_between_two_literal_runs_agrees_on_both_tiers() {
    // A hole with text on BOTH sides is three text nodes, and their ORDER is the whole content of
    // the sentence. Getting the pieces right and the order wrong still renders "right" to a
    // careless eye.
    assert_eq!(
        agree(
            "hole between literals",
            "## Component App\nState:\n    Let n be 2.\nView:\n    <p>You have {n} messages waiting</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>You have 2 messages waiting</p></body>"
    );
}

#[test]
fn a_loop_over_nothing_with_no_else_arm_renders_nothing_on_both_tiers() {
    // The `{else:}` arm is covered above. Without one, an empty collection must render the
    // container and NOTHING inside it — not a stray empty child, and not a skipped container.
    assert_eq!(
        agree(
            "empty loop, no else",
            "## Component App\nState:\n    Let xs be a new Seq of Text.\nView:\n    <ul>{for each x in xs:}<li>{x}</li>{end}</ul>\n\n## Main\nMount App.\n"
        ),
        "<body><ul></ul></body>"
    );
}

#[test]
fn nested_loops_agree_on_both_tiers() {
    // Two loops in one view, the inner reading the OUTER's binding. The inner loop's variable and
    // the outer's must not collide in the lowered function, which is a scoping question the two
    // tiers answer with completely different machinery.
    assert_eq!(
        agree(
            "nested loops",
            "## Component App\nState:\n    Let rows be [\"a\", \"b\"].\n    Let cols be [1, 2].\nView:\n    <table>{for each r in rows:}<tr>{for each c in cols:}<td>{r}{c}</td>{end}</tr>{end}</table>\n\n## Main\nMount App.\n"
        ),
        "<body><table><tr><td>a1</td><td>a2</td></tr><tr><td>b1</td><td>b2</td></tr></table></body>"
    );
}

#[test]
fn a_void_element_and_an_explicitly_closed_one_agree_on_both_tiers() {
    // `<br>` and `<br/>` are the same element written two ways. Whatever the recorder serializes
    // them as, the two spellings must not differ from each other or across tiers.
    let bare = agree(
        "void bare",
        "## Component App\nView:\n    <p>a<br>b</p>\n\n## Main\nMount App.\n",
    );
    let closed = agree(
        "void self-closed",
        "## Component App\nView:\n    <p>a<br/>b</p>\n\n## Main\nMount App.\n",
    );
    assert_eq!(bare, closed, "`<br>` and `<br/>` are the same element");
}

#[test]
fn a_conditional_attribute_renders_the_same_on_both_tiers() {
    // What this lock is for is AGREEMENT. The SEMANTICS changed deliberately here, which is
    // what the previous version of this comment said would happen: an HTML boolean attribute is
    // a PRESENCE, `disabled="false"` disables the control in every browser there is, and the
    // only encoding of "enabled" is no attribute at all. Both tiers now say so, through the one
    // rule in `dom::set_attribute_value`. `boolean_attribute_lock` holds the semantics; this
    // holds that the two tiers cannot drift apart on it.
    let page = |flag: &str| {
        format!(
            "## Component App\nState:\n    Let off be {flag}.\nView:\n    <button disabled={{off}}>go</button>\n\n## Main\nMount App.\n"
        )
    };
    assert_eq!(
        agree("disabled true", &page("true")),
        "<body><button disabled=\"\">go</button></body>"
    );
    assert_eq!(
        agree("disabled false", &page("false")),
        "<body><button>go</button></body>"
    );
}

#[test]
fn a_hole_holding_html_is_escaped_not_interpreted_on_both_tiers() {
    // The security floor, at the tier boundary: an ordinary hole is TEXT, whatever it contains.
    // `{raw}` is the only door to markup, and a page that renders a script tag from a plain hole
    // on one tier and escapes it on the other is a vulnerability that only appears in production.
    assert_eq!(
        agree(
            "hostile hole",
            "## Component App\nState:\n    Let t be \"<script>alert(1)</script>\".\nView:\n    <p>{t}</p>\n\n## Main\nMount App.\n"
        ),
        "<body><p>&lt;script&gt;alert(1)&lt;/script&gt;</p></body>"
    );
}

/// move from here into the agreement tests above, and this list shrinks.
/// What this tier still refuses, each entry standing on a gap that is REAL today.
///
/// ★★ THIS LIST HAS GONE STALE THREE TIMES, which is the lesson rather than the list. "A stateful
/// component placed twice" graduated when instances became keyed, "a styled component placed
/// twice" when the stylesheet moved to one-per-paint, and "a layout carrying `State:`" when
/// `{extends}` began drawing the layout as an instance of its own
/// (`a_layout_carrying_state_renders_the_same_on_both_tiers`). A catalog of NOT-YET is a moving
/// target by construction, and each time it empties there is a pull toward deleting it — which
/// would remove the rule that a tier limit must be LOUD rather than silent.
///
/// ★ THE ENTRY IS A DESIGN LIMIT RATHER THAN AN UNBUILT FEATURE. `{extends Site}` names a layout
/// and gives it nothing, so a layout that declares a prop has no value for it on any page that
/// extends it; this tier refuses the page by name rather than draw the layout with a hole.
const NOT_YET_ON_THE_VM: &[(&str, &str)] = &[
    ("a layout declaring a prop", "## Component Site (title: Text)\nView:\n    <header>site</header><main>{block content}</main>\n\n## Component App\nView:\n    {extends Site}{block content:}hi{end}\n\n## Main\nMount App.\n"),
];

#[test]
fn a_layout_carrying_state_renders_the_same_on_both_tiers() {
    assert_eq!(
        agree(
            "a layout carrying State:",
            "## Component Site\nState:\n    Let title be \"x\".\nView:\n    <header>{title}</header><main>{block content}</main>\n\n## Component App\nView:\n    {extends Site}{block content:}hi{end}\n\n## Main\nMount App.\n"
        ),
        "<body><header>x</header><main>hi</main></body>"
    );
}

#[test]
fn the_vm_refuses_what_it_cannot_yet_render() {
    for (name, src) in NOT_YET_ON_THE_VM {
        dom::reset();
        let out = vm_outcome(src);
        let error = out.error.unwrap_or_else(|| {
            panic!("`{name}` must be REFUSED by the VM, not silently run:\n{src}")
        });
        assert!(
            error.contains("tier"),
            "`{name}`'s refusal must say it is a tier limit: {error}"
        );
        assert_eq!(out.output, "", "a refused program produces no output");
    }
}

/// ★ `When a fetch fails (url: Text) and (why: Text):` — a page's ONE place to say what a failed
/// request means.
///
/// It needed no host import. A typed fetch already branched on its answer and already built an
/// error-carrying `Result` on both of its failure paths — a non-2xx status and a body that will
/// not decode — and both were telling only the action that asked. The sentence the language offers
/// for hearing about it was refused by name on every tier.
///
/// ★ AND IT IS AN OBSERVER, WHICH IS WHAT THIS TEST PINS. Both fields move: the action still gets
/// its `Err` and decides what THIS request means, and the handler sees every failure the component
/// has. Delivering only one of the two would silently break one of the two ways a page is written.
#[test]
fn a_failed_fetch_reaches_the_page_wide_handler() {
    // A status the server DID answer with; a body that will not decode; and a request that worked.
    for (label, status, body, want) in [
        (
            "a non-2xx",
            404,
            "missing",
            "<body><p id=\"a\">err</p><p id=\"b\">/api/todo</p></body>",
        ),
        (
            "a bad body",
            200,
            "not json",
            "<body><p id=\"a\">err</p><p id=\"b\">/api/todo</p></body>",
        ),
        (
            "a fetch that worked",
            200,
            r#"{"id":1,"text":"milk"}"#,
            "<body><p id=\"a\">milk</p><p id=\"b\">-</p></body>",
        ),
    ] {
        dom::reset();
        dom::stub_fetch("GET", "/api/todo", status, body);
        let out = tw_outcome(FETCH_PAGE);
        assert_eq!(out.error, None, "`{label}` must run cleanly");
        assert_eq!(
            dom::render(),
            want,
            "`{label}`: the action that asked and the page-wide handler must BOTH be told, and a \
             request that worked must leave the handler untouched"
        );
    }
}

/// A typed fetch completes on the bytecode runner exactly as it does on the reference tier: the
/// page RUNS, the request is sent, and the settle step calls the fetch's own completion — the
/// action that asked and the page-wide failure handler both see the same answer a browser would
/// give it (`runtime.js`'s `fetchSend` drives the identical lowering).
#[test]
fn a_typed_fetch_answers_the_same_on_both_tiers() {
    dom::reset();
    dom::stub_fetch("GET", "/api/todo", 404, "missing");
    let tw = tw_outcome(FETCH_PAGE);
    assert_eq!(tw.error, None);
    let tw_doc = dom::render();

    dom::reset();
    dom::stub_fetch("GET", "/api/todo", 404, "missing");
    let vm = vm_outcome(FETCH_PAGE);
    assert_eq!(vm.error, None);
    assert_eq!(
        dom::render(),
        tw_doc,
        "the VM must complete the fetch and answer identically"
    );
}

/// The page both fetch tests drive: an action that decides what THIS request meant, and a
/// page-wide handler that sees every failure.
const FETCH_PAGE: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n\
                          ## Component App\nState:\n    Let problem be \"-\".\n    Let failed be \"-\".\n\
                          Actions:\n    When the page loads:\n        Get \"/api/todo\" as Todo, then loaded.\n\
                          \x20   To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n\
                          \x20           When Ok (v):\n                Set problem to v's text.\n\
                          \x20           When Err (e):\n                Set problem to \"err\".\n\
                          \x20   When a fetch fails (url: Text) and (why: Text):\n        Set failed to url.\n\
                          View:\n    <p id=\"a\">{problem}</p><p id=\"b\">{failed}</p>\n\n## Main\nMount App.\n";

/// ★ `Set the title to "…".` — the sentence `docs/web.md` TEACHES BY NAME, now buildable.
///
/// The worst pairing a language can have: documentation hands someone a sentence, and the browser
/// tier refuses the whole component the moment they write it. The reference tier had always run it
/// (`dom::set_title`); what was missing was a builtin and one host import, `dom_set_title` — a
/// single concrete write to one page property, which is what the ABI's shim rule asks of an import.
///
/// The title is not in the rendered body, so this reads the recorder directly — a page that painted
/// identically while setting a different title is exactly the divergence a DOM comparison misses.
#[test]
fn setting_the_title_reaches_the_page_on_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        \
                       Set n to n + 1.\n        Set the title to \"Todos\".\nView:\n    \
                       <p id=\"n\">{n}</p><button id=\"b\" on-click={go}>b</button>\n\n\
                       ## Main\nMount App.\nClick \"#b\".\n";
    let read = |tier: &str, go: &dyn Fn(&str) -> Option<String>| {
        dom::reset();
        assert_eq!(go(SRC), None, "the {tier} must run it cleanly");
        (dom::render(), dom::title())
    };
    let tw = read("tree-walker", &|s| tw_outcome(s).error);
    let vm = read("VM", &|s| vm_outcome(s).error);
    assert_eq!(tw.1, "Todos", "the reference tier sets the document title");
    assert_eq!(
        tw, vm,
        "and the VM must set the same title AND paint the same page"
    );
}

/// ★ `Go back.` — the visitor's Back button, asked for by the page, on both tiers.
///
/// Both halves were missing, and the reason the reference tier had none is worth keeping: the
/// recorder logged what a page DID (`route_ops`, every `replaceState` included) and never modelled
/// where Back would LAND. Those are different questions — a `replaceState` swaps the entry you are
/// standing on and creates nothing to go back to — so going back got its own stack rather than a
/// reading of the op log.
///
/// The page navigates, then goes back, and reads the location each time. A `Go back.` that did
/// nothing would leave `/todos` on screen at the second stage, which is exactly the silent no-op a
/// navigation command must never be.
#[test]
fn going_back_returns_to_the_previous_page_on_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        \
                       Navigate to \"/todos\".\n    To back:\n        Go back.\nView:\n    \
                       <p id=\"p\">{routePath()}</p><button id=\"b\" on-click={go}>go</button>\
                       <button id=\"k\" on-click={back}>k</button>\n\n## Main\nMount App.\n";
    const CHROME: &str = "<button id=\"b\">go</button><button id=\"k\">k</button></body>";

    assert_eq!(
        agree("navigate away", &format!("{SRC}Click \"#b\".\n")),
        format!("<body><p id=\"p\">/todos</p>{CHROME}")
    );
    assert_eq!(
        agree(
            "and back again",
            &format!("{SRC}Click \"#b\".\nClick \"#k\".\n")
        ),
        format!("<body><p id=\"p\">/</p>{CHROME}"),
        "`Go back.` must land on the entry the page came from"
    );
    // ★ AND AT THE FIRST ENTRY IT DOES NOTHING, QUIETLY. A browser's Back button on a freshly
    // opened page leaves it alone; `Go back.` there is a request the history cannot satisfy, not
    // a mistake to refuse over.
    assert_eq!(
        agree("back with nowhere to go", &format!("{SRC}Click \"#k\".\n")),
        format!("<body><p id=\"p\">/</p>{CHROME}")
    );
}

/// ★ `Add class …` / `Remove class …` — and a class list is WORDS, not a substring.
///
/// The rule lives in compiled LOGOS (`classListWith` / `classListWithout` in web.lg) rather than in
/// a host function, so both tiers run the same one; the only import it needed is the generic
/// `getAttribute` read. Two host functions named `dom_add_class`/`dom_remove_class` would have been
/// fewer lines and would have put the rule in a .js file only a browser executes.
///
/// This drives the case a substring implementation gets wrong in BOTH directions: `active` already
/// contains `act`, so adding `act` must still happen, and removing `act` must leave `active` whole.
#[test]
fn a_class_is_added_and_removed_as_a_whole_word_on_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    To add:\n        \
                       Add class \"act\" to \"#c\".\n        Set n to n + 1.\n    To drop:\n        \
                       Remove class \"act\" from \"#c\".\n        Set n to n + 1.\nView:\n    \
                       <p id=\"c\" class=\"active\">{n}</p><button id=\"a\" on-click={add}>a</button>\
                       <button id=\"d\" on-click={drop}>d</button>\n\n## Main\nMount App.\n";
    const CHROME: &str = "<button id=\"a\">a</button><button id=\"d\">d</button></body>";
    // Attributes render in NAME order, not the order the view writes them.
    let page = |class: &str, n: i32| format!("<body><p class=\"{class}\" id=\"c\">{n}</p>{CHROME}");

    assert_eq!(
        agree(
            "added beside a class that contains it",
            &format!("{SRC}Click \"#a\".\n")
        ),
        page("active act", 1),
        "`act` must be added even though `active` already contains those three letters"
    );
    assert_eq!(
        agree(
            "removed without cutting its neighbour",
            &format!("{SRC}Click \"#a\".\nClick \"#d\".\n")
        ),
        page("active", 2),
        "removing `act` must leave `active` whole — a substring removal would maim it"
    );
    assert_eq!(
        agree(
            "added twice",
            &format!("{SRC}Click \"#a\".\nClick \"#a\".\n")
        ),
        page("active act", 2),
        "adding a class already present must not duplicate it"
    );
}

/// ★ AN IMPERATIVE DOM EFFECT SURVIVES THE TURN THAT MADE IT, on both tiers, whether or not the turn
/// moved state. A repaint commits only the differences between the paint before and the paint now,
/// and a class the view never wrote is in neither: the class `Add class` put on `#c` is not the
/// view's to take away, so the paint leaves it — with the turn's own change beside it when the turn
/// moved something the view shows.
#[test]
fn an_imperative_dom_write_survives_its_turn_whether_or_not_the_turn_changes_state() {
    const STILL: &str =
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To add:\n        \
                         Add class \"act\" to \"#c\".\nView:\n    \
                         <p id=\"c\">x</p><button id=\"a\" on-click={add}>a</button>\n\n\
                         ## Main\nMount App.\nClick \"#a\".\n";
    assert_eq!(
        agree("a class added by a turn that moved nothing", STILL),
        "<body><p class=\"act\" id=\"c\">x</p><button id=\"a\">a</button></body>",
        "the class stays on the element the turn put it on"
    );
    const MOVING: &str =
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To add:\n        \
                          Add class \"act\" to \"#c\".\n        Set n to n + 1.\nView:\n    \
                          <p id=\"c\">{n}</p><button id=\"a\" on-click={add}>a</button>\n\n\
                          ## Main\nMount App.\nClick \"#a\".\n";
    assert_eq!(
        agree("a class added by a turn that repaints the element", MOVING),
        "<body><p class=\"act\" id=\"c\">1</p><button id=\"a\">a</button></body>",
        "the repaint writes the turn's change and leaves the class beside it"
    );
}

/// ★ `When this component disappears:` — the unmount half of `Appears`, on both tiers, with NO
/// host import.
///
/// ★ IT IS A DIFF BETWEEN THE PAGE AND WHAT THE PROGRAM BELIEVES IS ON THE PAGE. Every component
/// that asks to be told keeps a map from instance key to the PAINT PASS it last rendered in
/// (`__SEEN_<C>`); a paint stamps everything it draws with the current pass, and afterwards any
/// instance still carrying an older number was not drawn — so it left. The DOM and the retained
/// instance maps linked, and the unmount set falls out of the comparison. Nothing had to be asked
/// of the host, and a page that never writes the sentence carries neither the map nor the sweep.
///
/// ★ AND IT MUST NOT PAINT. The instance is gone; a repaint would `clearChildren` the element it
/// used to occupy, which some other component may now own. The reference tier runs the body and
/// reads its fields back without painting, and the browser tier now does the same.
///
/// The handler sets the TITLE rather than touching the page, because the title is observable from
/// outside the tree — a DOM effect would be erased by the very repaint that caused the unmount.
#[test]
fn a_component_that_leaves_the_page_is_told_on_both_tiers() {
    const SRC: &str = "## Component Row\nState:\n    Let n be 0.\nActions:\n    \
                       When this component disappears:\n        Set the title to \"gone\".\nView:\n    \
                       <li id=\"r\">row</li>\n\n\
                       ## Component App\nState:\n    Let show be true.\nActions:\n    To hide:\n        \
                       Set show to false.\nView:\n    <ul>{if show:}<Row/>{end}</ul>\
                       <button id=\"b\" on-click={hide}>b</button>\n\n## Main\nMount App.\n";

    let read = |tier: &str, src: &str, go: &dyn Fn(&str) -> Option<String>| {
        dom::reset();
        assert_eq!(go(src), None, "the {tier} must run it cleanly:\n{src}");
        (dom::render(), dom::title())
    };

    // Still mounted: the row is there and nothing has been told anything.
    let tw = read("tree-walker", SRC, &|s| tw_outcome(s).error);
    let vm = read("VM", SRC, &|s| vm_outcome(s).error);
    assert_eq!(tw.1, "", "nothing has disappeared yet");
    assert_eq!(tw, vm, "and both tiers agree before the unmount");

    // The `{if}` closes, the row leaves, and the row's own sentence runs.
    let gone = format!("{SRC}Click \"#b\".\n");
    let tw = read("tree-walker", &gone, &|s| tw_outcome(s).error);
    let vm = read("VM", &gone, &|s| vm_outcome(s).error);
    assert_eq!(
        tw.1, "gone",
        "the row left the page, so its `When this component disappears:` must run"
    );
    assert!(
        !tw.0.contains("id=\"r\""),
        "and the row really is gone: {}",
        tw.0
    );
    assert_eq!(
        tw, vm,
        "and the VM must agree with the reference tier, page and title"
    );
}

/// ★ THE WINDOW EVENTS — `When the app goes offline:`, `… comes back online:`,
/// `When the window resizes:` — reach a page, and a test can cause one.
///
/// All three were in `Trigger::undeliverable`: NO tier ran them, so a page that wrote one got an
/// honest refusal at mount and nothing else. They are not things a visitor does to an ELEMENT and
/// not things a page does to itself, so none of the existing driving verbs could cause one — and a
/// trigger nothing can cause is a trigger nothing checks.
///
/// `Raise "offline".` is that driver, and it is ONE sentence for every event of this shape: the
/// event is named as text, so the next window event needs no new grammar. In a browser the same
/// handler is reached by `window.addEventListener` instead — one event, two routes in.
#[test]
fn a_window_event_reaches_its_handler_on_both_tiers() {
    for (label, trigger, raise) in [
        ("offline", "When the app goes offline:", "offline"),
        ("online", "When the app comes back online:", "online"),
        ("resize", "When the window resizes:", "resize"),
    ] {
        let src = format!(
            "## Component App\nState:\n    Let n be 0.\nActions:\n    {trigger}\n        \
             Set n to n + 1.\nView:\n    <p id=\"n\">{{n}}</p>\n\n## Main\nMount App.\nRaise \"{raise}\".\n"
        );
        assert_eq!(
            agree(label, &src),
            "<body><p id=\"n\">1</p></body>",
            "`{trigger}` must answer `Raise \"{raise}\".` on both tiers"
        );
    }

    // ★ AND ONLY ITS OWN EVENT. Three handlers on one page, one raise: exactly one must move, or
    // the name is decoration and every window trigger is really the same trigger.
    const ALL: &str =
        "## Component App\nState:\n    Let off be 0.\n    Let on be 0.\n    Let big be 0.\n\
                       Actions:\n    When the app goes offline:\n        Set off to off + 1.\n    \
                       When the app comes back online:\n        Set on to on + 1.\n    \
                       When the window resizes:\n        Set big to big + 1.\nView:\n    \
                       <p id=\"s\">{off}{on}{big}</p>\n\n## Main\nMount App.\n";
    assert_eq!(
        agree(
            "only the offline handler",
            &format!("{ALL}Raise \"offline\".\n")
        ),
        "<body><p id=\"s\">100</p></body>"
    );
    assert_eq!(
        agree(
            "only the resize handler",
            &format!("{ALL}Raise \"resize\".\n")
        ),
        "<body><p id=\"s\">001</p></body>"
    );

    // A raise nothing answers is not an error: a page may ignore the network coming back.
    assert_eq!(
        agree(
            "unanswered raise",
            "## Component App\nState:\n    Let n be 0.\nView:\n    <p id=\"n\">{n}</p>\n\n\
             ## Main\nMount App.\nRaise \"offline\".\n"
        ),
        "<body><p id=\"n\">0</p></body>"
    );
}

/// ★ `When the user scrolls to the bottom:` — the infinite-scroll sentence, on both tiers.
///
/// ★ THE SEMANTIC EVENT IS WHAT REACHES THE HANDLER. A browser raises `scroll` many times a second
/// and only some of those are "at the bottom", so the raw event is not what the page asked about.
/// The browser listens for `scroll` and asks the host for ONE number — how much content sits below
/// the fold (`dom_scroll_gap`) — and the threshold that turns that into "the bottom" is compiled
/// LOGOS, where every tier runs the same rule and an ordinary lock can read it.
///
/// What is dispatched, on every tier, is `scrolledtobottom`: an event that already means something.
/// A tier with no viewport can therefore cause it directly, which is what `Raise` does here — and
/// `Raise "…" on "#list".` names a scroll box rather than the page.
#[test]
fn scrolling_to_the_bottom_reaches_its_handler_on_both_tiers() {
    const PAGE: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    \
                        When the user scrolls to the bottom:\n        Set n to n + 1.\nView:\n    \
                        <p id=\"n\">{n}</p>\n\n## Main\nMount App.\n";
    assert_eq!(
        agree(
            "scrolled to the bottom",
            &format!("{PAGE}Raise \"scrolledtobottom\".\n")
        ),
        "<body><p id=\"n\">1</p></body>"
    );
    // Twice is twice: an infinite list loads another page each time it is reached.
    assert_eq!(
        agree(
            "twice",
            &format!("{PAGE}Raise \"scrolledtobottom\".\nRaise \"scrolledtobottom\".\n")
        ),
        "<body><p id=\"n\">2</p></body>"
    );

    // ★ AND A NAMED SCROLL BOX IS ITS OWN. `… of "#list":` listens on that element, so a page with
    // both must be able to tell them apart — otherwise the selector is decoration.
    const TWO: &str = "## Component App\nState:\n    Let page be 0.\n    Let list be 0.\nActions:\n    \
                       When the user scrolls to the bottom:\n        Set page to page + 1.\n    \
                       When the user scrolls to the bottom of \"#list\":\n        Set list to list + 1.\nView:\n    \
                       <div id=\"list\">rows</div><p id=\"s\">{page}{list}</p>\n\n## Main\nMount App.\n";
    assert_eq!(
        agree(
            "the page, not the list",
            &format!("{TWO}Raise \"scrolledtobottom\".\n")
        ),
        "<body><div id=\"list\">rows</div><p id=\"s\">10</p></body>"
    );
    assert_eq!(
        agree(
            "the list, not the page",
            &format!("{TWO}Raise \"scrolledtobottom\" on \"#list\".\n")
        ),
        "<body><div id=\"list\">rows</div><p id=\"s\">01</p></body>"
    );
}

/// ★ `Every … seconds:` and `After … seconds:` — a page's own clock.
///
/// ★ ONE IMPLEMENTATION, NOT TWO. A browser has real timers and a test has none, so the obvious
/// build gives the two tiers different code and hopes they agree. Instead every piece of "which
/// timer is due" is compiled LOGOS in `__lgx_tick`, and the tiers differ only in what CALLS it: a
/// browser arms ONE `setInterval` for the whole page — not one per timer — and hands it the
/// elapsed milliseconds, while `Advance time by …` calls the same function directly. That is also
/// why `Stop the … timer.` needed no import at all: a timer is a countdown held in a map, and
/// stopping it is writing the value that means "not running".
///
/// A repeating timer fires once per elapsed PERIOD rather than once per tick: 25 seconds of a
/// 10-second timer is two firings, and the 100 that follow are ten more — so a page told that a
/// minute passed does the same thing whether the clock arrived in one step or sixty.
///
/// ★ WHY THIS ONE IS NOT AN `agree(…)` TEST, stated rather than left as an absence. Two tier
/// limits stand between a timer and a cross-tier DOM comparison, and both are deliberate:
/// `Advance time by …` is parser-gated to `## Test` blocks, and a `## Test` block's page queries
/// (`the text of "#n"`) are refused on the bytecode tier by name — *"asks about the page, which
/// this tier does not have"*. So the reference tier is proved here, and the tier a visitor
/// actually uses is proved where it can be: `lgx_wasm_lock` compiles the page, and `wasm_dom`'s
/// `a_page_timer_fires_through_the_real_module` fires the page's interval closure through wasmi
/// and reads the ops back. That is stronger evidence than the in-process VM could give.
#[test]
fn a_timer_fires_once_per_elapsed_period() {
    const SRC: &str =
        "## Component App\nState:\n    Let n be 0.\n    Let once be 0.\nActions:\n    \
        Every 10 seconds as refresh:\n        Set n to n + 1.\n    \
        After 25 seconds as settle:\n        Set once to once + 1.\nView:\n    \
        <p id=\"n\">{n}</p><p id=\"o\">{once}</p>\n\n\
        ## Test: a clock the page keeps\n    Mount App.\n    Advance time by 25 seconds.\n    \
        Expect that the text of \"#n\" is equal to \"2\".\n    \
        Expect that the text of \"#o\" is equal to \"1\".\n    \
        Advance time by 100 seconds.\n    \
        Expect that the text of \"#n\" is equal to \"12\".\n    \
        Expect that the text of \"#o\" is equal to \"1\".\n";

    dom::reset();
    let report = logicaffeine_compile::testrun::run_tests(
        SRC,
        &logicaffeine_compile::testrun::TestConfig::default(),
    )
    .expect("the suite parses");
    assert_eq!(
        report.failed, 0,
        "a repeating timer fires once per elapsed period and a one-shot fires exactly once: \
         {report:?}"
    );
    assert_eq!(
        report.passed, 1,
        "and the test must actually have run: {report:?}"
    );
}

/// ★ `Stop the … timer.` — and it needed no host import.
///
/// A timer is a countdown held per instance; stopping it writes the value that means "not
/// running", and nothing re-arms it. The browser never armed a timer per handler to clear — it
/// arms one interval for the whole page — so there is no `clearInterval` in the ABI and no way for
/// the two halves to disagree about what "stopped" means.
#[test]
fn a_stopped_timer_stops() {
    const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    \
        Every 10 seconds as refresh:\n        Set n to n + 1.\n    \
        To halt:\n        Stop the timer refresh.\nView:\n    \
        <p id=\"n\">{n}</p><button id=\"b\" on-click={halt}>b</button>\n\n\
        ## Test: a clock that is told to stop\n    Mount App.\n    Advance time by 25 seconds.\n    \
        Expect that the text of \"#n\" is equal to \"2\".\n    Click \"#b\".\n    \
        Advance time by 100 seconds.\n    \
        Expect that the text of \"#n\" is equal to \"2\".\n";

    dom::reset();
    let report = logicaffeine_compile::testrun::run_tests(
        SRC,
        &logicaffeine_compile::testrun::TestConfig::default(),
    )
    .expect("the suite parses");
    assert_eq!(
        report.failed, 0,
        "a stopped timer must never fire again: {report:?}"
    );
    assert_eq!(
        report.passed, 1,
        "and the test must actually have run: {report:?}"
    );
}

/// ★ `When a file is dropped on "#zone" (name: Text):` — the last trigger in scope, on both tiers.
///
/// ★ THE SEMANTIC EVENT AGAIN, AND A VALUE WITH IT. A browser raises `drop` and the page must then
/// dig the file out of the transfer; what the sentence asked about is a file having been dropped,
/// WITH its name. So the browser's listener does the digging (one READ import,
/// `dom_event_file_name`) and calls the handler with the name, while what reaches the handler on
/// every tier is `filedropped`.
///
/// ★ AND THE DRIVER HAD TO LEARN TO CARRY A VALUE. A dropped file's name is not state and not a
/// prop — it arrives with the event — so a trigger that BINDS one cannot be caused by a sentence
/// with no way to say it. `Raise "…" on "…" with "…".` is that, and the dispatcher already had the
/// value slot: it is the same one a `Type … into` delivers a keystroke through.
#[test]
fn a_dropped_file_reaches_its_handler_with_its_name_on_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let got be \"-\".\nActions:\n    \
                       When a file is dropped on \"#zone\" (name: Text):\n        Set got to name.\nView:\n    \
                       <div id=\"zone\">drop here</div><p id=\"g\">{got}</p>\n\n## Main\nMount App.\n";
    const CHROME: &str = "<div id=\"zone\">drop here</div>";

    assert_eq!(
        agree(
            "a dropped file",
            &format!("{SRC}Raise \"filedropped\" on \"#zone\" with \"photo.png\".\n")
        ),
        format!("<body>{CHROME}<p id=\"g\">photo.png</p></body>"),
        "the handler must be told WHICH file — a drop with no name is a drop nobody can act on"
    );

    // ★ AND ONLY ON ITS OWN ZONE. Two drop targets on one page, one raise: a selector that did not
    // narrow the delivery would fire both and make the zone decorative.
    const TWO: &str = "## Component App\nState:\n    Let a be \"-\".\n    Let b be \"-\".\nActions:\n    \
                       When a file is dropped on \"#one\" (name: Text):\n        Set a to name.\n    \
                       When a file is dropped on \"#two\" (name: Text):\n        Set b to name.\nView:\n    \
                       <div id=\"one\">1</div><div id=\"two\">2</div><p id=\"s\">{a}{b}</p>\n\n## Main\nMount App.\n";
    assert_eq!(
        agree(
            "the second zone only",
            &format!("{TWO}Raise \"filedropped\" on \"#two\" with \"x.txt\".\n")
        ),
        "<body><div id=\"one\">1</div><div id=\"two\">2</div><p id=\"s\">-x.txt</p></body>"
    );
}

/// ★ `Focus "#draft".` and `Blur "#draft".` — the cursor moves, and the page hears about it.
///
/// Two halves, and the second is the one that is easy to forget: performing the DOM command is not
/// the same as the page being TOLD. A real browser raises `focus`/`blur` itself when `el.focus()`
/// is called, so a listening page is answered by the platform — the in-process tiers have no
/// platform, so the delivery is spelled out, through the SAME dispatcher a click goes through.
///
/// Neither requires an answer: putting the cursor in a field is a thing a page does, and a form
/// that focuses its own first input answers nothing at all.
#[test]
fn focus_and_blur_move_the_cursor_and_tell_the_page_on_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let log be \"\".\nActions:\n    \
                       To go:\n        Focus \"#draft\".\n    To stop:\n        Blur \"#draft\".\n    \
                       When \"#draft\" is focused:\n        Set log to log + \"F\".\n    \
                       When \"#draft\" is blurred:\n        Set log to log + \"B\".\nView:\n    \
                       <input id=\"draft\"><p id=\"l\">{log}</p>\
                       <button id=\"a\" on-click={go}>a</button><button id=\"b\" on-click={stop}>b</button>\n\n\
                       ## Main\nMount App.\n";
    const CHROME: &str = "<button id=\"a\">a</button><button id=\"b\">b</button></body>";
    let page = |log: &str| format!("<body><input id=\"draft\"><p id=\"l\">{log}</p>{CHROME}");

    assert_eq!(
        agree("focused", &format!("{SRC}Click \"#a\".\n")),
        page("F"),
        "`Focus` must deliver `focus` to whatever waits for it, not just move the cursor"
    );
    assert_eq!(
        agree(
            "focused then blurred",
            &format!("{SRC}Click \"#a\".\nClick \"#b\".\n")
        ),
        page("FB")
    );

    // ★ AND NEITHER NEEDS AN ANSWER. A page that focuses a field nothing listens to is ordinary —
    // failing there would make a form unable to put the cursor in its own first input.
    assert_eq!(
        agree(
            "focus with nothing listening",
            "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        \
             Focus \"#draft\".\n        Set n to n + 1.\nView:\n    <input id=\"draft\">\
             <p id=\"n\">{n}</p><button id=\"a\" on-click={go}>a</button>\n\n\
             ## Main\nMount App.\nClick \"#a\".\n"
        ),
        "<body><input id=\"draft\"><p id=\"n\">1</p><button id=\"a\">a</button></body>"
    );
}

/// ★ `Scroll to "#card".` and `Select the text in "#draft".` — neither existed on ANY tier.
///
/// They were refused everywhere, not merely in a browser, so a page that wrote one had nowhere to
/// run. They ride the element-command import with focus and blur, and they differ from those in one
/// way worth stating: neither raises an event the language offers a sentence for, so the command is
/// the whole of it — there is nothing to deliver afterwards.
///
/// ★ AND THEY ARE RECORDER STATE, NOT OPS. Scrolling an element into view and selecting its text do
/// not change the document; they are things a browser does TO a document that stays the same. So
/// the page renders identically before and after, and the question a test asks is "did the page
/// ask", which is the only part a tier with no viewport can honestly answer.
#[test]
fn scrolling_to_and_selecting_an_element_reach_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        \
                       Scroll to \"#card\".\n        Select the text in \"#draft\".\n        \
                       Set n to n + 1.\nView:\n    <div id=\"card\">c</div><input id=\"draft\">\
                       <p id=\"n\">{n}</p><button id=\"b\" on-click={go}>b</button>\n\n\
                       ## Main\nMount App.\nClick \"#b\".\n";

    let read = |tier: &str, go: &dyn Fn(&str) -> Option<String>| {
        dom::reset();
        assert_eq!(go(SRC), None, "the {tier} must run it cleanly");
        (dom::render(), dom::scrolled_to(), dom::selected())
    };
    let tw = read("tree-walker", &|s| tw_outcome(s).error);
    let vm = read("VM", &|s| vm_outcome(s).error);

    assert!(
        tw.1 >= 0,
        "the reference tier must record WHICH element was scrolled to"
    );
    assert!(tw.2 >= 0, "and which element's text was selected");
    assert_ne!(
        tw.1, tw.2,
        "they are different elements and must not collapse onto one handle"
    );
    assert_eq!(
        tw, vm,
        "and the VM must agree about the page AND about both — a command recorded on one tier and \
         not the other is a page that behaves differently where it counts"
    );
}

/// ★ `Hover "#card".` — one sentence, two readings, and WHERE it is written says which.
///
/// From a test it means "the visitor's pointer went here", so an element that answers nothing means
/// the test named the wrong one — the rule `Click` follows, and the reason this verb required an
/// answer at all. From inside an action it is a page raising `mouseover` on its own element, which
/// nothing need be listening for: a page cannot hover over itself, so demanding an answer there
/// would refuse the only sensible reading.
///
/// The command and the event are different words, which is the detail worth pinning: the page asks
/// its host to hover, and what a browser raises for that is `mouseover` — what a
/// `When "#card" is hovered:` sentence actually waits for.
#[test]
fn hovering_from_an_action_raises_mouseover_on_both_tiers() {
    assert_eq!(
        agree(
            "a page hovering its own element",
            "## Component App\nState:\n    Let seen be 0.\nActions:\n    To go:\n        \
             Hover \"#card\".\n    When \"#card\" is hovered:\n        Set seen to seen + 1.\nView:\n    \
             <div id=\"card\">c</div><p id=\"s\">{seen}</p><button id=\"b\" on-click={go}>b</button>\n\n\
             ## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><div id=\"card\">c</div><p id=\"s\">1</p><button id=\"b\">b</button></body>",
        "a hover from an action must reach a `When … is hovered:` sentence"
    );

    // ★ AND NOTHING NEED BE LISTENING. This is the half that separates the page reading from the
    // test reading: as a page action a hover nobody answers is ordinary, not a mistake.
    assert_eq!(
        agree(
            "nothing listening",
            "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        \
             Hover \"#card\".\n        Set n to n + 1.\nView:\n    <div id=\"card\">c</div>\
             <p id=\"n\">{n}</p><button id=\"b\" on-click={go}>b</button>\n\n\
             ## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><div id=\"card\">c</div><p id=\"n\">1</p><button id=\"b\">b</button></body>"
    );
}

/// ★ `Copy … to the clipboard.`, `Download … as "…".`, `Notify "…".` — three capabilities that had
/// no implementation on ANY tier.
///
/// ★ RECORDED, NOT PERFORMED, WHERE THERE IS NO BROWSER. A tier with no browser cannot put anything
/// on a real clipboard, hand the viewer a file, or raise a notification — but it can record that
/// the page ASKED, which is the part a test needs and the only part honest to claim. In a browser
/// each is attempted and each may be refused (a clipboard a browser blocks, a download the viewer
/// declines, a notification without consent); a refusal is not the page's failure, so none traps.
///
/// The downloads and notifications are LISTS because they can happen more than once and "did the
/// page notify" and "how many times" are different questions — a page that notifies twice is
/// usually a bug. The clipboard holds one thing by definition, so it is the last write.
#[test]
fn the_three_page_capabilities_are_recorded_identically_on_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let draft be \"notes\".\nActions:\n    To go:\n        \
                       Copy \"hello\" to the clipboard.\n        Download draft as \"out.txt\".\n        \
                       Notify \"Saved.\".\nView:\n    <button id=\"b\" on-click={go}>b</button>\n\n\
                       ## Main\nMount App.\nClick \"#b\".\n";

    let read = |tier: &str, go: &dyn Fn(&str) -> Option<String>| {
        dom::reset();
        assert_eq!(go(SRC), None, "the {tier} must run it cleanly");
        (dom::clipboard(), dom::downloads(), dom::notifications())
    };
    let tw = read("tree-walker", &|s| tw_outcome(s).error);
    let vm = read("VM", &|s| vm_outcome(s).error);

    assert_eq!(tw.0, "hello", "the clipboard holds what the page put there");
    assert_eq!(
        tw.1,
        vec![("out.txt".to_string(), "notes".to_string())],
        "a download carries BOTH its name and its content — a file offered under the wrong name is \
         as wrong as the wrong bytes"
    );
    assert_eq!(tw.2.len(), 1, "one notification, once");
    assert_eq!(tw.2[0].0, "Saved.");
    assert_eq!(
        tw, vm,
        "and the VM must agree about all three — a capability recorded on one tier and not the \
         other is a page that behaves differently where it counts"
    );
}

/// ★ `Reload the page.` — everything goes, on both tiers, and the ORDER is the whole story.
///
/// This was the last verb in the catalogue, refused for a real reason: a turn writes its state back
/// at the END, so clearing every instance mid-body meant the write-back re-created the one it had
/// just cleared — carrying exactly the values the reload existed to forget. The page came back from
/// the dead on one tier and stayed dead on the other.
///
/// What closed it was not new machinery but the deferral focus and blur already needed, one step
/// further: the reload runs AFTER the write-back, and the paint is skipped, because a page that has
/// been torn down has nothing to draw. A reload is the one act that ends a turn rather than
/// concluding it.
#[test]
fn reloading_the_page_forgets_everything_on_both_tiers() {
    const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        \
                       Set n to n + 1.\n    To wipe:\n        Set n to 99.\n        Reload the page.\nView:\n    \
                       <p id=\"n\">{n}</p><button id=\"b\" on-click={go}>b</button>\
                       <button id=\"r\" on-click={wipe}>r</button>\n\n## Main\nMount App.\n";
    const CHROME: &str = "<button id=\"b\">b</button><button id=\"r\">r</button></body>";

    // The counter moves first, so a reload that did nothing would leave 1 on screen and be caught.
    assert_eq!(
        agree(
            "a click before the reload",
            &format!("{SRC}Click \"#b\".\n")
        ),
        format!("<body><p id=\"n\">1</p>{CHROME}")
    );

    // ★ AND THE TURN'S OWN WRITE-BACK MUST NOT RESURRECT IT. `wipe` sets `n` to 99 BEFORE reloading
    // — so a reload that ran mid-body would be undone by the write-back that follows, and the page
    // would come back showing 99. An empty document is the only answer that says the page is gone.
    assert_eq!(
        agree(
            "and then the reload",
            &format!("{SRC}Click \"#b\".\nClick \"#r\".\n")
        ),
        "<body></body>",
        "a reload takes the page with it — every instance, listener and mounted root"
    );
}

/// ★ A WRAPPER WITH ITS OWN STATE IS AN INSTANCE, NOT A TEXTUAL INLINING.
///
/// `<Card><p>hi</p></Card>` — the reference tier runs `Card` as a real component instance, with
/// its own `State:` bound, and moves the caller's children in at `{children}` (rendered in the
/// CALLER's scope, which is what they were written against). A tier that instead pastes Card's
/// markup into the caller leaves Card's own state unbound, so `{title}` reads whatever the CALLER
/// happens to have by that name — a different page, silently.
#[test]
fn a_wrapper_binds_its_own_state_on_both_tiers() {
    assert_eq!(
        agree(
            "wrapper state",
            "## Component Card\n\
             View:\n\x20   <div class=\"card\"><header>{title}</header>{children}</div>\n\
             State:\n\x20   Let title be \"Notes\".\n\n\
             ## Component App\n\
             View:\n\x20   <main><Card><p>hi</p></Card></main>\n\n\
             ## Main\nMount App.\n"
        ),
        "<body><main><div class=\"card\"><header>Notes</header><p>hi</p></div></main></body>"
    );
}

/// ★ A WRAPPER REPAINTS WITHOUT LOSING WHAT IT WRAPS.
///
/// The card's own state changes, so the card redraws — and its children came from the CALLER, in
/// the caller's scope, moved into the slot on the way in. A tier that repaints the card alone
/// redraws the frame around nothing, because the holder those children arrived in was emptied the
/// first time. The reference tier never had the bug: it repaints from the root, so the caller
/// re-renders its children every time. This pins the two to the same answer.
#[test]
fn a_wrapper_keeps_its_children_across_its_own_repaint() {
    assert_eq!(
        agree(
            "wrapper repaint",
            "## Component Card\n\
             View:\n\x20   <div class=\"card\"><b>{n}</b>{children}<button on-click={bump}>+</button></div>\n\
             State:\n\x20   Let n be 0.\n\
             Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\n\
             ## Component App\n\
             View:\n\x20   <main><Card><p>hi</p></Card></main>\n\n\
             ## Main\nMount App.\nClick \"button\".\n"
        ),
        "<body><main><div class=\"card\"><b>1</b><p>hi</p><button>+</button></div></main></body>"
    );
}

/// ★ A WRAPPER MOUNTED AS THE ROOT DRAWS AN EMPTY SLOT — and, more importantly, COMPILES.
///
/// A wrapper repaints through the page root rather than alone, because its children came from a
/// caller. A wrapper that IS the root has no caller, and an earlier version of that redirect did
/// not notice: it sent the repaint to some other unplaced component, and two such wrappers would
/// have redirected to each other forever — a compiler that never finishes, which no rendering test
/// would ever have reached. A root is mounted, so nothing hands it children and its slot is empty.
#[test]
fn a_wrapper_mounted_as_the_root_draws_an_empty_slot() {
    assert_eq!(
        agree(
            "root wrapper",
            "## Component Other\nView:\n\x20   <i>{children}</i>\n\n\
             ## Component App\n\
             View:\n\x20   <main><span>x</span>{children}</main>\n\n\
             ## Main\nMount App.\n"
        ),
        "<body><main><span>x</span></main></body>"
    );
}

/// ★ A WRAPPER INSIDE A WRAPPER'S CHILDREN. The inner placement's children are drawn while the
/// outer one's are still being built, so each needs a holder of its own — one shared name would
/// have the inner render overwrite the markup the outer was halfway through.
#[test]
fn wrappers_nest_without_overwriting_each_other() {
    assert_eq!(
        agree(
            "nested wrappers",
            "## Component Box (label: Text)\n\
             View:\n\x20   <div class=\"{label}\">{children}</div>\n\n\
             ## Component App\n\
             View:\n\x20   <Box label=\"out\"><Box label=\"in\"><p>deep</p></Box><b>after</b></Box>\n\n\
             ## Main\nMount App.\n"
        ),
        "<body><div class=\"out\"><div class=\"in\"><p>deep</p></div><b>after</b></div></body>"
    );
}

/// ★ A WRAPPER MOUNTED SOMEWHERE OTHER THAN `body` REPAINTS INTO THE RIGHT PLACE.
///
/// A wrapper's turn repaints the PAGE rather than itself, and every read a repaint makes goes
/// through one door holding `__lgx_key`. Inside the wrapper's turn that identifier is the
/// WRAPPER's key — so the redirected repaint looks the page up under the card's name, against a
/// map holding only the root's `""`.
///
/// ★ IT TAKES THREE THINGS AT ONCE TO SEE, which is why the first two versions of this test were
/// worthless. The state change must be UNPATCHABLE, or the turn never repaints and the redirect
/// never runs — here `{n}` sits inside a placement, whose node addresses are opaque, so the
/// in-place update cannot address it and the whole page is redrawn. And the root must be mounted
/// somewhere OTHER than the body, because `__M_App` is seeded `mapOf("", 0)` and node 0 IS the
/// body: look the page up under the wrong key on an ordinary `Mount App.` page and the wrong
/// answer is the right node.
#[test]
fn a_wrapper_repaints_a_root_that_is_not_the_body() {
    assert_eq!(
        agree(
            "wrapper repaint off-body",
            "## Component Box\n\
             View:\n\x20   <div class=\"box\">{children}</div>\n\n\
             ## Component Panel\n\
             View:\n\x20   <Box><b>{n}</b>{children}<button on-click={bump}>+</button></Box>\n\
             State:\n\x20   Let n be 0.\n\
             Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\n\
             ## Component App\n\
             View:\n\x20   <main><Panel><p>kept</p></Panel></main>\n\n\
             ## Main\n\
             Let root be createElement(\"div\").\n\
             Call setAttribute with root and \"id\" and \"side\".\n\
             Call appendChild with body() and root.\n\
             Mount App at \"#side\".\n\
             Click \"button\".\n"
        ),
        "<body><div id=\"side\"><main><div class=\"box\"><b>1</b><p>kept</p><button>+</button></div></main></div></body>"
    );
}

/// ★ A SLOT PASSED STRAIGHT THROUGH TO ANOTHER WRAPPER still makes this component a wrapper.
///
/// `View: <Box>{children}</Box>` — `Panel` draws nothing of its own except the box it wraps and
/// the children it forwards. Its `{children}` is nested inside a PLACEMENT rather than an element,
/// and the walk that decides "does this component take children" only descended through elements,
/// `{if}`, `{for each}` and `{inspect}`. So `Panel` was not on the wrapper list, kept the
/// per-component repaint it cannot take, and dropped what it wraps on its own first state change.
#[test]
fn a_slot_forwarded_through_another_wrapper_still_repaints_whole() {
    assert_eq!(
        agree(
            "forwarded slot",
            "## Component Box\n\
             View:\n\x20   <div class=\"box\">{children}</div>\n\n\
             ## Component Panel\n\
             View:\n\x20   <Box><b>{n}</b>{children}<button on-click={bump}>+</button></Box>\n\
             State:\n\x20   Let n be 0.\n\
             Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\n\
             ## Component App\n\
             View:\n\x20   <main><Panel><p>kept</p></Panel></main>\n\n\
             ## Main\nMount App.\nClick \"button\".\n"
        ),
        "<body><main><div class=\"box\"><b>1</b><p>kept</p><button>+</button></div></main></body>"
    );
}

/// ★ THE LISTENER CAN SIT TWO PLACEMENTS DEEP INSIDE TWO GENUINELY LISTENER-FREE WRAPPERS.
///
/// `Frame` and `Box` each declare no `State:`, no `Actions:` and no `on-*`/`bind` of their own —
/// `has_any_listener_site` is correctly `false` for both, and neither ever calls
/// `register_listener_with`. `App`'s OWN placement of `<Frame>` hands it a `<Box>` placement,
/// which in turn is handed the button — the button's `on-click={bump}` is lowered into `App`'s
/// OWN render function (`slot_argument` keeps `owner`/`out` as the PLACER's, all the way down),
/// so `App` is the one whose `has_any_listener_site` must see it, through TWO nested
/// `ViewNode::Component { children }` hops rather than one.
#[test]
fn a_listener_two_placements_deep_inside_listener_free_wrappers_still_registers() {
    assert_eq!(
        agree(
            "listener two wrappers deep",
            "## Component Box\n\
             View:\n\x20   <div class=\"box\">{children}</div>\n\n\
             ## Component Frame\n\
             View:\n\x20   <section>{children}</section>\n\n\
             ## Component App\n\
             View:\n\x20   <main><Frame><Box><b>{n}</b><button on-click={bump}>+</button></Box>\
             </Frame></main>\n\
             State:\n\x20   Let n be 0.\n\
             Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\n\
             ## Main\nMount App.\nClick \"button\".\n"
        ),
        "<body><main><section><div class=\"box\"><b>1</b><button>+</button></div></section></main></body>"
    );
}

/// ★ A COMPONENT REACTS TO A STORE, on both tiers — `When Session's name changes:`.
///
/// This is how shared state is used: one component calls a store action, and every component
/// watching that store hears about it. It compiled for the browser only after the census was
/// taught to see it at all (`FieldChanges { store: Option<…> }` is ONE variant carrying two
/// sentences with two different tier answers, so a per-variant census counted it once).
///
/// ★ AND COMPILING IS NOT RUNNING. The catalogue proves the page BUILDS for a browser; only this
/// proves the handler fires, on the right instance, with the store's new value visible to it.
#[test]
fn a_store_change_reaches_a_watching_component_on_both_tiers() {
    assert_eq!(
        agree(
            "store watch",
            "## Store Session\n\
             State:\n\x20   Let name be \"guest\".\n\
             Actions:\n\x20   To login (who: Text):\n\x20       Set name to who.\n\n\
             ## Component App\n\
             Use Session.\n\
             State:\n\x20   Let seen be 0.\n\
             Actions:\n\x20   When Session's name changes:\n\x20       Set seen to seen + 1.\n\
             \x20   To go:\n\x20       Call Session's login with \"ada\".\n\
             View:\n\x20   <main><p id=\"n\">{Session's name}</p><p id=\"s\">{seen}</p>\
             <button id=\"b\" on-click={go}>go</button></main>\n\n\
             ## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><main><p id=\"n\">ada</p><p id=\"s\">1</p><button id=\"b\">go</button></main></body>"
    );
}

/// ★ AND IT DOES NOT FIRE WHEN NOTHING MOVED. A watcher runs only for a field in the CHANGED set,
/// so a store action that writes the value already there must wake nobody. A tier that notified on every call would look correct in the test above
/// and re-run every watcher on the page for each store call — the spurious-rerun failure a
/// reactive framework is judged by, and one that only shows when the value is unchanged.
#[test]
fn a_store_write_that_changes_nothing_wakes_nobody() {
    assert_eq!(
        agree(
            "store watch unchanged",
            "## Store Session\n\
             State:\n\x20   Let name be \"guest\".\n\
             Actions:\n\x20   To login (who: Text):\n\x20       Set name to who.\n\n\
             ## Component App\n\
             Use Session.\n\
             State:\n\x20   Let seen be 0.\n\
             Actions:\n\x20   When Session's name changes:\n\x20       Set seen to seen + 1.\n\
             \x20   To go:\n\x20       Call Session's login with \"guest\".\n\
             View:\n\x20   <main><p id=\"s\">{seen}</p><button id=\"b\" on-click={go}>go</button></main>\n\n\
             ## Main\nMount App.\nClick \"#b\".\n"
        ),
        "<body><main><p id=\"s\">0</p><button id=\"b\">go</button></main></body>"
    );
}

/// ★ A MESSAGE ON A TOPIC REACHES ITS HANDLER — the last trigger, on both tiers.
///
/// `When a message arrives on "chat" (m: Text):` is a SUBSCRIPTION: a component says which topic
/// it cares about and receives what was published there. It is independent of a
/// `## Store … shared on "…"`, which is a different construct with a different refusal — this
/// trigger needs a transport, not a replica protocol, and the relay
/// (`logicaffeine_system::relay`, with its `Subscribe`/`Publish`/`Event` frames and a browser
/// WebSocket client) was already built.
///
/// ★ AND A TEST CAN CAUSE ONE WITH THE VERB THAT ALREADY EXISTS. A subscription registers as a
/// page-level listener for `message:<topic>`, so `Raise … on "body" with …` delivers it — the same
/// machinery `Raise "offline"` uses. `event_round_trip_lock`'s rule (every event a handler waits
/// for, a test must be able to cause) is met without inventing a driver.
#[test]
fn a_message_on_a_topic_reaches_its_handler_on_both_tiers() {
    assert_eq!(
        agree(
            "message",
            "## Component App\n\
             State:\n\x20   Let last be \"none\".\n\
             Actions:\n\x20   When a message arrives on \"chat\" (m: Text):\n\
             \x20       Set last to m.\n\
             View:\n\x20   <main><p id=\"l\">{last}</p></main>\n\n\
             ## Main\nMount App.\nRaise \"message:chat\" on \"body\" with \"hello\".\n"
        ),
        "<body><main><p id=\"l\">hello</p></main></body>"
    );
}

/// ★ AND A MESSAGE ON A DIFFERENT TOPIC DOES NOT. A subscription that received everything would
/// pass the test above and be useless — the topic is the whole point of a topic.
#[test]
fn a_message_on_another_topic_leaves_the_handler_alone() {
    assert_eq!(
        agree(
            "message other topic",
            "## Component App\n\
             State:\n\x20   Let last be \"none\".\n\
             Actions:\n\x20   When a message arrives on \"chat\" (m: Text):\n\
             \x20       Set last to m.\n\
             View:\n\x20   <main><p id=\"l\">{last}</p></main>\n\n\
             ## Main\nMount App.\nRaise \"message:other\" on \"body\" with \"hello\".\n"
        ),
        "<body><main><p id=\"l\">none</p></main></body>"
    );
}

/// ★ AND A PAGE CAN SEND ONE. `When a message arrives on …:` shipped with no way to CAUSE a
/// message from inside a page — the inbound half of a construct, landing without its outbound
/// half, which is the shape under most of the bugs this campaign has found.
///
/// The relay had the whole other side already: `RelayFrame::Publish` delivers to every local
/// subscriber and is gossiped on to native nodes (`logicaffeine_system::relay`). What was missing
/// was a SENTENCE. Nothing could be published from LGX, so a page could hear a server and never
/// answer it, and two tabs of the same app could not talk at all.
///
/// ★ THE LOCAL LEG IS COMPILED LOGOS, NOT THE HOST'S JOB. `Publish M on "t"` dispatches
/// `message:t` through the page's own subscription table — the same path `Raise` drives — and
/// THEN hands the frame to the transport. So the page's own handlers run identically on every
/// tier with no transport at all, and the host import carries only the outward leg. That is the
/// shim rule honoured rather than worked around: the deciding stays in the module.
#[test]
fn a_page_can_publish_a_message_to_its_own_subscriber_on_both_tiers() {
    assert_eq!(
        agree(
            "publish",
            "## Component App\n\
             State:\n\x20   Let last be \"none\".\n\
             Actions:\n\x20   When a message arrives on \"chat\" (m: Text):\n\
             \x20       Set last to m.\n\
             \x20   To announce:\n\x20       Publish \"hi\" on \"chat\".\n\
             View:\n\x20   <main><p id=\"l\">{last}</p>\
             <button id=\"a\" on-click={announce}>a</button></main>\n\n\
             ## Main\nMount App.\nClick \"#a\".\n"
        ),
        "<body><main><p id=\"l\">hi</p><button id=\"a\">a</button></main></body>"
    );
}

/// ★ AND THE TOPIC IS STILL THE POINT. A publish that reached every subscriber would pass the
/// test above and be useless.
#[test]
fn a_publish_on_another_topic_leaves_the_handler_alone() {
    assert_eq!(
        agree(
            "publish other topic",
            "## Component App\n\
             State:\n\x20   Let last be \"none\".\n\
             Actions:\n\x20   When a message arrives on \"chat\" (m: Text):\n\
             \x20       Set last to m.\n\
             \x20   To announce:\n\x20       Publish \"hi\" on \"news\".\n\
             View:\n\x20   <main><p id=\"l\">{last}</p>\
             <button id=\"a\" on-click={announce}>a</button></main>\n\n\
             ## Main\nMount App.\nClick \"#a\".\n"
        ),
        "<body><main><p id=\"l\">none</p><button id=\"a\">a</button></main></body>"
    );
}

/// ★ AND WHAT IT SENDS IS A VALUE, NOT A LITERAL. A publish that could only carry text written at
/// the call site could not report anything the page actually knows, which is the only reason to
/// publish at all.
#[test]
fn a_publish_carries_a_state_field_on_both_tiers() {
    assert_eq!(
        agree(
            "publish state",
            "## Component App\n\
             State:\n\x20   Let draft be \"ada\".\n\x20   Let last be \"none\".\n\
             Actions:\n\x20   When a message arrives on \"chat\" (m: Text):\n\
             \x20       Set last to m.\n\
             \x20   To announce:\n\x20       Publish draft on \"chat\".\n\
             View:\n\x20   <main><p id=\"l\">{last}</p>\
             <button id=\"a\" on-click={announce}>a</button></main>\n\n\
             ## Main\nMount App.\nClick \"#a\".\n"
        ),
        "<body><main><p id=\"l\">ada</p><button id=\"a\">a</button></main></body>"
    );
}

/// ★ A PUBLISH IS DELIVERED AFTER THE TURN THAT SENT IT, and the case that proves it is the
/// turn that changes NOTHING — which is the opposite of what it looks like.
///
/// The obvious reading is that a nested turn is lost to its parent's WRITE-BACK: the parent read
/// its state into locals before the child ran, so committing them afterwards would overwrite what
/// the child set. By that reading a parent which also assigns a field is the dangerous one.
///
/// Measured with the deferral removed, it is the reverse. A parent that assigns something PATCHES
/// — only the sites whose values changed are redrawn — so a field the child wrote is not among
/// them and survives. A parent that assigns nothing has an empty dirty set, which the turn's paint
/// reads as "I know of no change" and answers with a FULL REPAINT drawn from the view, erasing
/// the child's paint entirely. The same property
/// `an_imperative_dom_write_is_lost_when_its_turn_changes_no_state` pins for an imperative DOM
/// write governs a nested turn's paint, and it means a publishing action with a `Set` in it is
/// the case that CANNOT fail.
///
/// So both shapes are here, and the first is the one with teeth: `a_page_can_publish_…` above
/// publishes from an action that assigns nothing, and goes red the moment either tier delivers
/// inline. This one holds the other half of the rule, so that a future change which makes the
/// patching parent lossy too is not silently fine.
#[test]
fn a_publish_reaches_its_handler_whether_or_not_the_sending_turn_wrote_state() {
    for (label, body, sent) in [
        (
            "assigns nothing",
            "\x20       Publish \"hi\" on \"chat\".\n",
            "0",
        ),
        (
            "assigns a field",
            "\x20       Set sent to sent + 1.\n\x20       Publish \"hi\" on \"chat\".\n",
            "1",
        ),
    ] {
        let src = format!(
            "## Component App\nState:\n\x20   Let last be \"none\".\n\x20   Let sent be 0.\n\
             Actions:\n\x20   When a message arrives on \"chat\" (m: Text):\n\
             \x20       Set last to m.\n\x20   To announce:\n{body}\
             View:\n\x20   <main><p id=\"l\">{{last}}</p><p id=\"s\">{{sent}}</p>\
             <button id=\"a\" on-click={{announce}}>a</button></main>\n\n\
             ## Main\nMount App.\nClick \"#a\".\n"
        );
        assert_eq!(
            agree(&format!("publish, {label}"), &src),
            format!(
                "<body><main><p id=\"l\">hi</p><p id=\"s\">{sent}</p>\
                 <button id=\"a\">a</button></main></body>"
            )
        );
    }
}

/// ★ AND THE FRAME MUST ACTUALLY LEAVE THE PAGE, on both tiers.
///
/// Every test above watches the LOCAL leg — the page's own subscriber — which is exactly the half
/// that shows up in rendered HTML. A publish that delivered locally and sent nothing would satisfy
/// all of them while two tabs of the app still could not talk, which is the failure this verb was
/// added to remove. The outward leg is only observable as a record that a frame was handed over,
/// so both tiers keep one and this reads it back.
#[test]
fn a_publish_hands_the_same_frame_to_the_transport_on_both_tiers() {
    let src = "## Component App\n\
               State:\n\x20   Let draft be \"ada\".\n\
               Actions:\n\x20   To announce:\n\x20       Publish draft on \"chat\".\n\
               View:\n\x20   <main><button id=\"a\" on-click={announce}>a</button></main>\n\n\
               ## Main\nMount App.\nClick \"#a\".\n";
    dom::reset();
    assert_eq!(tw_outcome(src).error, None);
    let tw = dom::published();
    dom::reset();
    assert_eq!(vm_outcome(src).error, None);
    let vm = dom::published();
    assert_eq!(
        tw,
        vec![("chat".to_string(), "ada".to_string())],
        "the tree-walker must send it"
    );
    assert_eq!(tw, vm, "both tiers must hand the transport the same frame");
}

/// ★ AN APP-SHAPED PAGE THAT USES THE WHOLE REALTIME SEAM AT ONCE.
///
/// Per-construct locks cannot see interactions, and every bug this campaign found in its last
/// stretch came from a page that looked like something a person would build rather than a page
/// that isolates one feature. This one is a chat: a `## Store` with an action, a component
/// WATCHING that store, a propped child inside a `{for each}`, a scoped `Style:`, a `bind`
/// input, and both directions of the relay — `Publish` from two different actions and a
/// subscription that accumulates what arrives.
///
/// It pins BOTH observable surfaces. The HTML is the local leg (the page's own subscriber ran,
/// twice, in order) and the transport record is the outward leg (two frames, in the same order,
/// carrying a bound input's value and a literal). A page that delivered locally and sent nothing
/// would satisfy the first assertion completely.
///
/// ★ AND IT FOUND NOTHING, which is worth recording rather than quietly dropping. Five earlier
/// probes of this shape each found a real bug on the first run; this one was byte-identical on
/// both tiers at the first attempt. The instrument is kept because a clean result from it is
/// evidence, and because the page is now a regression test for every seam it crosses.
#[test]
fn a_realtime_chat_page_agrees_on_both_tiers() {
    let src = "## Store Room\nState:\n\x20   Let members be 0.\n\x20   Let topic be \"general\".\n\
\x20Actions:\n\x20   To join:\n\x20       Set members to members + 1.\n\n\
## Component Bubble (who: Text, said: Text)\n\
Style:\n\x20   .bubble { padding: 4px; }\n\
View:\n\x20   <li class=\"bubble\"><b>{who}</b>: {said}</li>\n\n\
## Component Roster\nUse Room.\nState:\n\x20   Let seen be 0.\n\
Actions:\n\x20   When Room's members changes:\n\x20       Set seen to seen + 1.\n\
View:\n\x20   <aside id=\"r\">{seen} in {Room's topic}</aside>\n\n\
## Component Chat\nUse Room.\n\
State:\n\x20   Let draft be \"\".\n\x20   Let log be \"\".\n\x20   Let count be 0.\n\
Actions:\n\x20   When a message arrives on \"general\" (m: Text):\n\
\x20       Set log to log + m + \";\".\n\x20       Set count to count + 1.\n\
\x20   To send:\n\x20       Publish draft on \"general\".\n\x20       Call Room's join.\n\
\x20   To announce:\n\x20       Publish \"hi\" on \"general\".\n\
View:\n\x20   <main>\n\
\x20       <ul>{for each w in [\"ada\", \"grace\"]:}<Bubble who=\"{w}\" said=\"hello\"/>{end}</ul>\n\
\x20       <p id=\"log\">{log}</p>\n\x20       <p id=\"n\">{count}</p>\n\
\x20       <input id=\"d\" bind={draft}/>\n\
\x20       <button id=\"s\" on-click={send}>send</button>\n\
\x20       <button id=\"a\" on-click={announce}>announce</button>\n\
\x20       <Roster/>\n\x20   </main>\n\n\
## Main\nMount Chat.\nType \"hello there\" into \"#d\".\nClick \"#s\".\nClick \"#a\".\n";
    assert_eq!(
        agree("realtime chat", src),
        "<body><style>.bubble.lgx-bubble { padding: 4px; }</style>\
<main><ul><li class=\"bubble lgx-bubble\"><b class=\"lgx-bubble\">ada</b>: hello</li>\
<li class=\"bubble lgx-bubble\"><b class=\"lgx-bubble\">grace</b>: hello</li></ul>\
<p id=\"log\">hello there;hi;</p><p id=\"n\">2</p>\
<input id=\"d\" value=\"hello there\"><button id=\"s\">send</button>\
<button id=\"a\">announce</button><aside id=\"r\">1 in general</aside></main></body>"
    );
    assert_eq!(
        dom::published(),
        vec![
            ("general".to_string(), "hello there".to_string()),
            ("general".to_string(), "hi".to_string()),
        ],
        "both publishes must reach the transport, in order"
    );
}

/// ★ AND A PAGE THAT PUBLISHES ON A TOPIC NOBODY HEARS IS NOT AN ERROR. A publish is addressed to
/// whoever is listening, which may be another tab or a native node and may be nobody at all — the
/// same judgement `Raise` makes, and the opposite of `Click`, where naming an element nothing
/// answers means the test named the wrong element.
#[test]
fn publishing_where_nothing_subscribes_is_not_an_error() {
    let src = "## Component App\n\
               Actions:\n\x20   To announce:\n\x20       Publish \"hi\" on \"nobody\".\n\
               View:\n\x20   <main><button id=\"a\" on-click={announce}>a</button></main>\n\n\
               ## Main\nMount App.\nClick \"#a\".\n";
    assert_eq!(
        agree("publish unheard", src),
        "<body><main><button id=\"a\">a</button></main></body>"
    );
    assert_eq!(
        dom::published(),
        vec![("nobody".to_string(), "hi".to_string())]
    );
}

/// ★ A STYLESHEET LANDS IN THE SAME PLACE ON BOTH TIERS, and not inside a list.
///
/// The sheet used to be appended by whichever instance rendered FIRST, so a styled row drawn in a
/// `<ul>` put its `<style>` between the list and its first item — invalid content model, and
/// foster-parented straight out of a `<table>`. The server had it worse: no once-per-component
/// guard at all, so the rules were served once PER ROW.
///
/// Both tiers now emit every reachable sheet at the mount point, ahead of the markup. This pins
/// the DOM half; `ssr_subset_lock` pins the served half, and they were written to agree.
#[test]
fn a_stylesheet_lands_at_the_mount_point_on_both_tiers() {
    assert_eq!(
        agree(
            "sheet placement",
            "## Component Row (label: Text)\n\
             Style:\n\x20   .row { color: red; }\n\
             View:\n\x20   <li class=\"row\">{label}</li>\n\n\
             ## Component App\n\
             View:\n\x20   <ul>{for each x in [\"a\", \"b\"]:}<Row label={x} />{end}</ul>\n\n\
             ## Main\nMount App.\n"
        ),
        "<body><style>.row.lgx-row { color: red; }</style>\
         <ul><li class=\"row lgx-row\">a</li><li class=\"row lgx-row\">b</li></ul></body>"
    );
}

/// ★ A STORE IS NOT A PAGE — and electing one as the paint root loses the page's stylesheets.
///
/// The paint root is "the component nobody places", used to decide who emits the page's `<style>`
/// blocks and where a wrapper's repaint redirects. A `## Store` is unplaced and propless, so on any
/// page with one the STORE was elected — and a store never renders, so the sheets were never
/// emitted. The served page carried its CSS and the painted page did not.
///
/// ★ NEITHER FEATURE ALONE SHOWS IT. Every wrapper test passed (no store); every store test passed
/// (no wrapper, no `Style:`). It took one page holding a store, a styled wrapper, a component
/// watching that store, a `{for each}`, an `{inspect}` and a timer — which is simply what a real
/// page looks like.
#[test]
fn a_store_does_not_get_elected_as_the_page_root() {
    assert_eq!(agree("store + wrapper + style", "## Store Feed\nState:\n    Let unread be 0.\nActions:\n    To arrive:\n        Set unread to unread + 1.\n\n## Component Panel (title: Text)\nStyle:\n    .panel { border: 1px solid #ccc; }\nView:\n    <section class=\"panel\"><h2>{title}</h2>{children}</section>\n\n## Component Chip (n: Int)\nUse Feed.\nState:\n    Let seen be 0.\nActions:\n    When Feed's unread changes:\n        Set seen to seen + 1.\nView:\n    <span class=\"chip\">{n}/{seen}</span>\n\n## Component App\nUse Feed.\nState:\n    Let picked be none.\n    Let ticks be 0.\nActions:\n    Every 10 seconds:\n        Set ticks to ticks + 1.\n    To push:\n        Call Feed's arrive.\n    To choose:\n        Set picked to some 42.\nView:\n    <main>\n        <Panel title=\"Live\">\n            <ul>\n                {for each i in [1, 2]:}\n                    <li><Chip n={i} /></li>\n                {end}\n            </ul>\n            {inspect picked:}\n                {when Some (v):}<p id=\"p\">picked {v}</p>\n                {when None:}<p id=\"p\">none</p>\n            {end}\n            <p id=\"t\">{ticks}</p>\n        </Panel>\n        <button id=\"push\" on-click={push}>push</button>\n        <button id=\"ch\" on-click={choose}>choose</button>\n    </main>\n\n## Main\nMount App.\nClick \"#push\".\nClick \"#ch\".\n"), "<body><style>.panel.lgx-panel { border: 1px solid #ccc; }</style><main><section class=\"panel lgx-panel\"><h2 class=\"lgx-panel\">Live</h2><ul><li><span class=\"chip\">1/1</span></li><li><span class=\"chip\">2/1</span></li></ul><p id=\"p\">picked 42</p><p id=\"t\">0</p></section><button id=\"push\">push</button><button id=\"ch\">choose</button></main></body>");
}
