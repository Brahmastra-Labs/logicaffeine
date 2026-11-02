//! ★ LOCK — a `## Directive` is the extension point for element behavior: a plain attribute in
//! any template that runs LOGOS against the real element. ★
//!
//! This is how a user adds a capability to the framework without a compiler change. A directive
//! receives the `Element` it sits on and the attribute's value; it runs when the element is
//! created, again whenever the value changes, and its `When removed:` block runs when the
//! element leaves the page.

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

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

const TOOLTIP: &str = "\
## Directive tooltip (el: Element) and (text: Text):
    Call setAttribute with el and \"title\" and text.
";

#[test]
fn a_directive_runs_against_the_element_it_sits_on() {
    let src = format!(
        "{TOOLTIP}\n## Component App\nState:\n    Let hint be \"delete this\".\nView:\n    <button tooltip={{hint}}>x</button>\n\n## Main\nMount App.\n"
    );
    assert_eq!(rendered(&src), "<button title=\"delete this\">x</button>");
}

#[test]
fn a_directive_sees_the_value_of_its_expression_not_its_text() {
    let src = format!(
        "{TOOLTIP}\n## Component App\nState:\n    Let a be \"one\".\n    Let b be \"two\".\nView:\n    <button tooltip={{a + \" and \" + b}}>x</button>\n\n## Main\nMount App.\n"
    );
    assert_eq!(rendered(&src), "<button title=\"one and two\">x</button>");
}

#[test]
fn a_directive_runs_again_when_its_value_changes() {
    let src = format!(
        "{TOOLTIP}\n## Component App\nState:\n    Let hint be \"before\".\nActions:\n    To change:\n        Set hint to \"after\".\nView:\n    <button id=\"go\" tooltip={{hint}} on-click={{change}}>x</button>\n\n## Main\nMount App.\nClick \"#go\".\n"
    );
    assert!(
        rendered(&src).contains("title=\"after\""),
        "rendered: {}",
        rendered(&src)
    );
}

#[test]
fn a_directive_is_not_rendered_as_an_attribute_itself() {
    let src = format!(
        "{TOOLTIP}\n## Component App\nState:\n    Let hint be \"hi\".\nView:\n    <button tooltip={{hint}}>x</button>\n\n## Main\nMount App.\n"
    );
    assert!(
        !rendered(&src).contains("tooltip="),
        "the directive is behavior, not markup: {}",
        rendered(&src)
    );
}

#[test]
fn a_directives_removed_block_runs_when_the_element_leaves() {
    let src = "\
## Directive watched (el: Element) and (label: Text):
    Show \"mounted \" + label.
    When removed:
        Show \"removed \" + label.

## Component App
State:
    Let showing be true.
Actions:
    To hide:
        Set showing to false.
View:
    <button id=\"go\" on-click={hide}>x</button>
    {if showing:}<p watched=\"a\">here</p>{end}

## Main
Mount App.
Click \"#go\".
";
    assert_eq!(output(src), ["mounted a", "removed a"]);
    assert!(
        !rendered(src).contains("<p"),
        "the element is gone: {}",
        rendered(src)
    );
}

#[test]
fn a_directive_may_take_only_the_element() {
    let src = "\
## Directive focused (el: Element):
    Call setAttribute with el and \"data-focused\" and \"yes\".

## Component App
View:
    <input id=\"draft\" focused=\"\">

## Main
Mount App.
";
    assert!(
        rendered(src).contains("data-focused=\"yes\""),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn an_undeclared_directive_stays_a_plain_attribute() {
    // Only DECLARED names are directives; anything else is ordinary markup, so a template can
    // still write `data-*`, `aria-*` and any attribute the platform grows next.
    let src = "## Component App\nState:\n    Let v be \"x\".\nView:\n    <p data-kind={v}>t</p>\n\n## Main\nMount App.\n";
    assert_eq!(rendered(src), "<p data-kind=\"x\">t</p>");
}

#[test]
fn a_directive_that_traps_names_itself() {
    let src = "\
## Directive bad (el: Element) and (v: Text):
    Call setAttribute with el and \"title\" and item 9 of [\"a\"].

## Component App
State:
    Let v be \"x\".
View:
    <p bad={v}>t</p>

## Main
Mount App.
";
    let msg = failure(src);
    assert!(
        msg.contains("bad"),
        "the message names the directive: {msg}"
    );
}

// ── the lowered page: the same lifecycle, compiled ────────────────────────────────────────────────

/// `src` on the lowered VM: its printed lines and its document.
fn lowered(src: &str) -> (Vec<String>, String) {
    dom::reset();
    let out = logicaffeine_compile::compile::vm_outcome(src);
    assert_eq!(out.error, None, "the lowered page must run cleanly:\n{src}");
    (
        out.output.lines().map(str::to_string).collect(),
        dom::render(),
    )
}

fn builds_for_the_browser(src: &str) {
    let built = logicaffeine_compile::compile::compile_to_wasm(src);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

#[test]
fn a_directive_that_takes_only_the_element_runs_on_the_lowered_vm() {
    let src = "\
## Directive focused (el: Element):
    Call setAttribute with el and \"data-focused\" and \"yes\".

## Component App
View:
    <input id=\"draft\" focused=\"\">

## Main
Mount App.
";
    let (_, doc) = lowered(src);
    assert!(doc.contains("data-focused=\"yes\""), "{doc}");
    builds_for_the_browser(src);
}

const WATCHED: &str = "\
## Directive watched (el: Element) and (label: Text):
    Show \"mounted \" + label.
    When removed:
        Show \"removed \" + label.
";

#[test]
fn a_directives_removed_block_runs_when_its_element_leaves_on_the_lowered_vm() {
    let src = format!(
        "{WATCHED}
## Component App
State:
    Let showing be true.
    Let n be 0.
Actions:
    To hide:
        Set showing to false.
    To bump:
        Set n to n + 1.
View:
    <button id=\"go\" on-click={{hide}}>x</button>
    <button id=\"b\" on-click={{bump}}>{{n}}</button>
    {{if showing:}}<p watched=\"a\">here</p>{{end}}

## Main
Mount App.
Click \"#b\".
Click \"#go\".
Click \"#b\".
"
    );
    let (lines, doc) = lowered(&src);
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("removed")).cloned().collect::<Vec<_>>(),
        ["removed a"],
        "the removed block runs once, when the element leaves — not on a repaint that keeps it: {lines:?}"
    );
    assert!(!doc.contains("<p"), "the element is gone: {doc}");
    builds_for_the_browser(&src);
}

#[test]
fn a_leaving_components_directives_are_removed_with_it_on_the_lowered_vm() {
    let src = format!(
        "{WATCHED}
## Component Row (label: Text)
View:
    <li watched={{label}}>{{label}}</li>

## Component App
State:
    Let items be [\"x\", \"y\"].
Actions:
    To drop:
        Set items to [\"y\"].
View:
    <button id=\"go\" on-click={{drop}}>x</button>
    <ul>{{for each t in items, keyed by t:}}<Row label={{t}}/>{{end}}</ul>

## Main
Mount App.
Click \"#go\".
"
    );
    let (lines, _) = lowered(&src);
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("removed")).cloned().collect::<Vec<_>>(),
        ["removed x"],
        "the row that left takes its directive with it, and the row that stayed keeps its own: {lines:?}"
    );
    builds_for_the_browser(&src);
}

#[test]
fn a_placed_components_own_repaint_removes_only_its_own_directives_on_the_lowered_vm() {
    let src = format!(
        "{WATCHED}
## Component Card (name: Text)
State:
    Let open be true.
Actions:
    To toggle:
        Set open to not open.
View:
    <div><button id=\"{{name}}\" on-click={{toggle}}>t</button>{{if open:}}<b watched={{name}}>b</b>{{end}}</div>

## Component App
View:
    <main><Card name=\"one\"/><Card name=\"two\"/></main>

## Main
Mount App.
Click \"#one\".
"
    );
    let (lines, _) = lowered(&src);
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("removed"))
            .cloned()
            .collect::<Vec<_>>(),
        ["removed one"],
        "card one's repaint removes its own directive and leaves card two's: {lines:?}"
    );
    builds_for_the_browser(&src);
}

/// `src` on the lowered VM: the program's failure.
fn lowered_failure(src: &str) -> String {
    dom::reset();
    logicaffeine_compile::compile::vm_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected the lowered page to fail:\n{src}"))
}

/// The failure a page's `## Test` records on the lowered tree-walker.
fn lowered_here_failure(src: &str) -> String {
    use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
    let mut all = compare_tests(src, &[PageTierKind::LoweredHere], 0, 50_000_000, None)
        .unwrap_or_else(|e| panic!("the page must parse: {e}\n{src}"));
    let c = all.remove(0);
    // Once the lowered page is the reference, `compare_tests` no longer lists it as a candidate.
    let run = c
        .candidates
        .into_iter()
        .find(|(kind, _)| *kind == PageTierKind::LoweredHere)
        .map(|(_, run)| run)
        .or(Some(TierRun::Traced(c.reference)));
    match run {
        Some(TierRun::Traced(t)) => t
            .stopped
            .map(|s| s.message)
            .or_else(|| {
                t.steps
                    .iter()
                    .rev()
                    .find_map(|s| s.snapshot.failure.clone())
            })
            .unwrap_or_else(|| panic!("expected the lowered page to fail:\n{src}")),
        other => panic!("the lowered tree-walker must trace the page, not {other:?}"),
    }
}

const TRAPS_IN_BLOCK: &str = "\
## Directive bad (el: Element) and (v: Text):
    Call setAttribute with el and \"title\" and item 9 of [\"a\"].

## Component App
State:
    Let v be \"x\".
View:
    <p bad={v}>t</p>
";

/// ★ THE LOWERED PAGE NAMES THE DIRECTIVE TOO, in the reference's sentence: its block opens the
/// page's failure context, and whatever catches the trap puts the context in front of it.
#[test]
fn a_directive_that_traps_names_itself_on_the_lowered_page() {
    let prefix = logicaffeine_language::ast::component::directive_failure_prefix("bad", false);
    let main = format!("{TRAPS_IN_BLOCK}\n## Main\nMount App.\n");
    let vm = lowered_failure(&main);
    assert!(
        vm.starts_with(&prefix),
        "the lowered VM names the directive: {vm}"
    );
    let here = lowered_here_failure(&format!(
        "{TRAPS_IN_BLOCK}\n## Test: it traps\n    Mount App.\n"
    ));
    assert!(
        here.starts_with(&prefix),
        "the lowered tree-walker names the directive: {here}"
    );
    builds_for_the_browser(&main);
}

/// A `When removed:` block that traps is named as that block.
#[test]
fn a_removed_block_that_traps_names_itself_on_the_lowered_page() {
    let src = "\
## Directive bad (el: Element) and (v: Text):
    Show \"on\".
    When removed:
        Show item 9 of [\"a\"].

## Component App
State:
    Let open be true.
Actions:
    To toggle:
        Set open to not open.
View:
    <main>{if open:}<p bad=\"x\">t</p>{end}<button id=\"t\" on-click={toggle}>t</button></main>

## Main
Mount App.
Click \"#t\".
";
    let prefix = logicaffeine_language::ast::component::directive_failure_prefix("bad", true);
    let vm = lowered_failure(src);
    assert!(vm.starts_with(&prefix), "the removed block is named: {vm}");
    let reference = failure(src);
    assert!(
        reference.starts_with(&prefix),
        "as the reference names it: {reference}"
    );
}

/// The context closes with the block: a trap after a directive ran cleanly is not named by it.
#[test]
fn a_trap_outside_a_directive_is_not_named_by_one() {
    let src = "\
## Directive fine (el: Element) and (v: Text):
    Call setAttribute with el and \"title\" and v.

## Component App
State:
    Let v be \"x\".
    Let n be 0.
Actions:
    To boom:
        Set n to item 9 of [1].
View:
    <p fine={v}>t</p><button id=\"b\" on-click={boom}>b</button>

## Main
Mount App.
Click \"#b\".
";
    let vm = lowered_failure(src);
    assert!(
        !vm.contains("directive"),
        "an action's trap names no directive: {vm}"
    );
    assert!(vm.contains("out of bounds"), "{vm}");
}
