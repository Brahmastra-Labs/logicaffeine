//! LOCK — `{for each x in xs, at index i:}` counts from ONE, on every tier.
//!
//! Every other position in LOGOS is 1-based: `item 1 of xs` is the first item, `item 0 of xs` is
//! an error. A template index is a position in the same list, so it names the same numbers — the
//! row that shows `{i}` is the row `item i of xs` reads. Four walks draw a template (the reference
//! tier's DOM recorder, the bytecode VM's lowered render, and the server's `html of` walk on both
//! of those engines) and every one of them is held to the same numbers here. The compiled wasm
//! module paints the same numbers: `wasm_dom::an_lgx_loop_index_counts_from_one_on_all_three_tiers`.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// The document `src` paints on the tree-walker.
fn tw_page(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "the tree-walker must run it cleanly:\n{src}"
    );
    dom::render()
}

/// The document `src` paints on the bytecode VM.
fn vm_page(src: &str) -> String {
    dom::reset();
    let out = vm_outcome(src);
    assert_eq!(out.error, None, "the VM must run it cleanly:\n{src}");
    dom::render()
}

/// The painted document, held equal across the two engines.
fn painted(src: &str) -> String {
    let tw = tw_page(src);
    let vm = vm_page(src);
    assert_eq!(tw, vm, "the two engines paint different documents:\n{src}");
    tw
}

/// The markup the server walk (`html of App`) produces, held equal across the two engines.
fn served(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must serve it:\n{src}");
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must serve it:\n{src}");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        vm.output.lines().collect::<Vec<_>>(),
        "the two engines serve different markup:\n{src}"
    );
    tw.output.trim().to_string()
}

/// A page whose `View:` is `view` over `State:` `state`, mounted.
fn mounted(state: &str, view: &str) -> String {
    format!("## Component App\nState:\n{state}View:\n    {view}\n\n## Main\nMount App.\n")
}

/// The same page, rendered by the server walk.
fn server(state: &str, view: &str) -> String {
    format!("## Component App\nState:\n{state}View:\n    {view}\n\n## Main\nShow html of App.\n")
}

/// Every walk, one answer: the painted body and the served markup are the same document.
fn every_tier(state: &str, view: &str) -> String {
    let body = painted(&mounted(state, view));
    let body = body
        .strip_prefix("<body>")
        .and_then(|b| b.strip_suffix("</body>"))
        .unwrap_or(&body)
        .to_string();
    let html = served(&server(state, view));
    assert_eq!(
        body, html,
        "the painted and the served document differ for:\n{view}"
    );
    body
}

const ABC: &str = "    Let xs be [\"a\", \"b\", \"c\"].\n";

#[test]
fn a_loop_index_starts_at_one_on_every_tier() {
    assert_eq!(
        every_tier(
            ABC,
            "<ul>{for each x in xs, at index i:}<li>{i}:{x}</li>{end}</ul>"
        ),
        "<ul><li>1:a</li><li>2:b</li><li>3:c</li></ul>"
    );
}

#[test]
fn the_index_names_the_item_it_counts() {
    // The property the base exists for: `item i of xs` is the row's own item, on every tier.
    assert_eq!(
        every_tier(
            ABC,
            "<ul>{for each x in xs, at index i:}<li>{x}={item i of xs}</li>{end}</ul>"
        ),
        "<ul><li>a=a</li><li>b=b</li><li>c=c</li></ul>"
    );
}

#[test]
fn the_last_index_is_the_length() {
    assert_eq!(
        every_tier(
            ABC,
            "<ul>{for each x in xs, at index i:}{if i is equal to length of xs:}<li>last {x}</li>{end}{end}</ul>"
        ),
        "<ul><li>last c</li></ul>"
    );
}

#[test]
fn a_keyed_loop_counts_from_one_too() {
    assert_eq!(
        every_tier(
            ABC,
            "<ul>{for each x in xs, keyed by x, at index i:}<li>{i}{x}</li>{end}</ul>"
        ),
        "<ul><li>1a</li><li>2b</li><li>3c</li></ul>"
    );
}

#[test]
fn an_index_feeds_arithmetic_and_attributes() {
    assert_eq!(
        every_tier(
            ABC,
            "<ul>{for each x in xs, at index i:}<li id=\"r{i}\">{i * 10}</li>{end}</ul>"
        ),
        "<ul><li id=\"r1\">10</li><li id=\"r2\">20</li><li id=\"r3\">30</li></ul>"
    );
}

#[test]
fn nested_loops_count_from_one_independently() {
    let state = "    Let rows be [[\"a\", \"b\"], [\"c\"]].\n";
    assert_eq!(
        every_tier(
            state,
            "<ol>{for each r in rows, at index i:}<li>{for each c in r, at index j:}[{i}.{j}{c}]{end}</li>{end}</ol>"
        ),
        "<ol><li>[1.1a][1.2b]</li><li>[2.1c]</li></ol>"
    );
}

#[test]
fn an_empty_indexed_loop_takes_its_else_arm() {
    let state = "    Let xs be a new Seq of Text.\n";
    assert_eq!(
        every_tier(
            state,
            "<ul>{for each x in xs, at index i:}<li>{i}</li>{else:}<li>none</li>{end}</ul>"
        ),
        "<ul><li>none</li></ul>"
    );
}

#[test]
fn a_one_item_loop_shows_one() {
    let state = "    Let xs be [\"solo\"].\n";
    assert_eq!(
        every_tier(
            state,
            "<ul>{for each x in xs, at index i:}<li>{i}{x}</li>{end}</ul>"
        ),
        "<ul><li>1solo</li></ul>"
    );
}

#[test]
fn a_row_action_given_the_index_moves_that_row() {
    // The index an action receives is the index `item i of xs` answers to: clicking the second
    // button changes the second item, on both engines.
    let src = "## Component App\nState:\n    Let xs be [\"a\", \"b\", \"c\"].\nActions:\n    To pick (at: Int):\n        Set item at of xs to \"X\".\nView:\n    <ul>{for each x in xs, at index i:}<li><button id=\"b{i}\" on-click={pick(i)}>{x}</button></li>{end}</ul>\n\n## Main\nMount App.\nClick \"#b2\".\n";
    assert_eq!(
        painted(src),
        "<body><ul><li><button id=\"b1\">a</button></li><li><button id=\"b2\">X</button></li><li><button id=\"b3\">c</button></li></ul></body>"
    );
}

#[test]
fn stateful_rows_in_an_indexed_loop_stay_distinct() {
    // The loop position also tells one iteration's child instance from the next one's; counting
    // from one must not collapse two rows onto one record.
    let src = "## Component Row (label: Text)\nState:\n    Let clicks be 0.\nActions:\n    To bump:\n        Set clicks to clicks + 1.\nView:\n    <li class=\"row\" on-click={bump}>{label}{clicks}</li>\n\n## Component App\nState:\n    Let xs be [\"a\", \"b\"].\nView:\n    <ul>{for each x in xs, at index i:}<Row label={x}/>{end}</ul>\n\n## Main\nMount App.\nClick \".row\".\n";
    assert_eq!(
        painted(src),
        "<body><ul><li class=\"row\">a1</li><li class=\"row\">b0</li></ul></body>"
    );
}

#[test]
fn the_server_walk_with_an_index_builds_on_the_deploy_tier() {
    // AOT-Rust is the tier a served page ships on and the only one that type-checks the walk;
    // `Ok` from `compile_to_rust` is not enough, since a refusal comes back as a `compile_error!`.
    let src = server(
        ABC,
        "<ul>{for each x in xs, at index i:}<li>{i}:{x}={item i of xs}</li>{end}</ul>",
    );
    let rust =
        compile_to_rust(&src).unwrap_or_else(|e| panic!("must compile to AOT-Rust: {e:?}\n{src}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "the generated Rust would not build:\n{}",
        refusals.join("\n")
    );
}
