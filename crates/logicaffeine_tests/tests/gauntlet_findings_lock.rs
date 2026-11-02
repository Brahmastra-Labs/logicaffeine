//! LOCK — friction points hit building the Wave 8 gauntlet (real apps from `docs/` alone), each a
//! finding under its own heading below. RED until the
//! owning lane lands the fix; do not edit these fixtures to make them pass — fix the compiler.
//!
//! Every fixture below is the SMALLEST page that reproduces the finding, reduced from a real app
//! (`examples/todo`'s persistence, which needed a documented "persisted across reloads" list and
//! hit all three).

use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

#[path = "compile_common/mod.rs"]
mod common;
#[path = "common/divergent_page.rs"]
mod divergent_page;

/// G9 — `.stop` (real: `crates/logicaffeine_language/src/lgx/html.rs`'s `Behaviour::Stop`, and
/// undocumented in `docs/frontend.md`/`LOGOS_QUICKGUIDE.md`) refuses the WHOLE PAGE for the browser
/// backend, confirming the campaign's own plan's older finding ("`.stop`/`.once`/`.capture`/
/// `.passive` refused in browsers") is still open. The refusal message itself is good — this test
/// pins that the CONSTRUCT eventually builds, not that the message improves.
#[test]
fn stop_event_modifier_builds_for_the_browser() {
    use logicaffeine_compile::compile::compile_to_wasm;

    let source = "\
## Page /

Actions:
    To swallow:
        Return.

    To close:
        Return.

View:
    <div id=\"backdrop\" on-click={close}>
      <div id=\"modal\" on-click.stop={swallow}>content</div>
    </div>
";
    let result = compile_to_wasm(source);
    assert!(
        result.is_ok(),
        "`.stop` is a real, documented-in-source event modifier that runs correctly on the \
         tree-walker; a page using it must build for the browser: {:?}",
        result.err()
    );
}

/// G13 — `When this component appears:` runs before its own component's markup reaches the DOM: a
/// DOM-querying verb (`Focus`) aimed at the component's OWN element fails, though the identical verb
/// aimed at an element OUTSIDE the newly-mounted subtree (proven by the sibling test below) works.
#[test]
fn a_component_can_focus_its_own_element_on_appearing() {
    const FOCUS_OWN_ELEMENT: &str = r##"
## Component Box

Actions:
    When this component appears:
        Focus "#inner".

View:
    <button id="inner">inner</button>

## Page /

State:
    Let shown be false.

Actions:
    To toggle:
        Set shown to true.

View:
    <button id="outer" on-click={toggle}>outer</button>
    {if shown:}<Box/>{end}

## Test: focusing the newly mounted component's own element

Click "#outer".
Expect that "#inner" is focused.
"##;
    let outcome = one_outcome(FOCUS_OWN_ELEMENT);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "`When this component appears:` must be able to focus (or otherwise DOM-query) its OWN \
         just-mounted element — the hook's whole documented purpose is reacting to the component \
         having appeared: {:?}",
        outcome.failures
    );
}

/// The control proving G13 is specifically about the NEWLY MOUNTED subtree, not about `Focus` or
/// `When this component appears:` in general: the identical construct, focusing an element OUTSIDE
/// the subtree that is appearing, already works today.
#[test]
fn a_component_can_already_focus_an_element_outside_its_own_subtree_on_appearing() {
    const FOCUS_OUTSIDE_ELEMENT: &str = r##"
## Component Box

Actions:
    When this component appears:
        Focus "#outer".

View:
    <button id="inner">inner</button>

## Page /

State:
    Let shown be false.

Actions:
    To toggle:
        Set shown to true.

View:
    <button id="outer" on-click={toggle}>outer</button>
    {if shown:}<Box/>{end}

## Test: focusing an element outside the newly mounted subtree

Click "#outer".
Expect that "#outer" is focused.
"##;
    let outcome = one_outcome(FOCUS_OUTSIDE_ELEMENT);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "if this control is failing, G13 is not what it looks like — re-diagnose before trusting \
         the sibling test above: {:?}",
        outcome.failures
    );
}

/// G14 — `Advance time by 1 millisecond.` (the English-word duration) is refused; the compact
/// literal `100ms` is not. Minor: found only while building the G13 workaround (`After 100ms:`
/// needs a matching sub-second advance in its own test).
#[test]
fn advance_time_accepts_the_word_form_of_a_sub_second_duration() {
    const WORD_FORM_MILLISECOND: &str = r##"
## Page /

State:
    Let focused be false.

Actions:
    After 1 millisecond:
        Set focused to true.

View:
    <p id="state">{focused}</p>

## Test: a sub-second English-word duration advances time

Advance time by 1 millisecond.
Expect that the text of "#state" is equal to "true".
"##;
    let outcome = one_outcome(WORD_FORM_MILLISECOND);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "`Advance time by 1 millisecond.` (the English-word spelling LOGOS uses for every other \
         duration in `docs/frontend.md`) must advance time exactly as `Advance time by 100ms.` \
         already does: {:?}",
        outcome.failures
    );
}

/// G17 — `examples/signup_form` compiles to an INVALID WASM module (a real WASM engine refuses to
/// even load it) while `largo check`/`largo test`/`largo build --static`/`largo test --browser` all
/// report success. `browser_tier_lock`'s corpus sweep (which actually loads every example's
/// compiled module in a real browser) caught it: `call[2] expected type i32, found local.get of
/// type i64` in function #20 — `compile_to_wasm` returning `Ok(bytes)` only means the compiler's
/// own encoding did not hit a KNOWN-unsupported construct; it is not proof the bytes are valid
/// WebAssembly, and nothing in the ordinary `largo` command surface (nor, per G1, an ordinary
/// auto-mounted page's own `## Test`) ever asks a real engine to load them before this lock does.
/// Owner: heap3.
#[test]
fn signup_forms_compiled_module_is_valid_wasm_a_real_engine_can_load() {
    use logicaffeine_compile::compile::compile_to_wasm;

    let source = include_str!("../../../examples/signup_form/pages/app.lg");
    let module = compile_to_wasm(source).expect("largo check/build already report this compiles");
    let engine = wasmi::Engine::default();
    let result = wasmi::Module::new(&engine, &module[..]);
    assert!(
        result.is_ok(),
        "examples/signup_form's compiled module must be valid WebAssembly a real engine can \
         instantiate — `largo build --static` reporting success is not proof of that, and this \
         exact page shipped a module `browser_tier_lock`'s real-browser sweep refuses to load \
         (\"call[2] expected type i32, found local.get of type i64\" in function #20): {result:?}"
    );
}

/// G17b — `examples/fetch_dashboard` hits the SAME class of bug as G17 (an invalid WASM module
/// heap3's fix for `signup_form`'s function #20 did not cover): `browser_tier_lock` refuses all 5
/// scenarios with a real wasm compile error at a DIFFERENT site, function #19 —
/// `local.set[0] expected type i32, found local.get of type i64`. Owner: heap3.
#[test]
fn fetch_dashboards_compiled_module_is_valid_wasm_a_real_engine_can_load() {
    use logicaffeine_compile::compile::compile_to_wasm;

    let source = include_str!("fixtures/lgx_gauntlet_pending/fetch_dashboard/pages/app.lg");
    let module = compile_to_wasm(source).expect("largo test already reports this compiles");
    let engine = wasmi::Engine::default();
    let result = wasmi::Module::new(&engine, &module[..]);
    assert!(
        result.is_ok(),
        "examples/fetch_dashboard's compiled module must be valid WebAssembly a real engine can \
         instantiate — `browser_tier_lock`'s real-browser sweep refuses to load it \
         (\"local.set[0] expected type i32, found local.get of type i64\" in function #19): {result:?}"
    );
}

/// G15 — a component's callback prop (`fn() -> Unit`, `docs/frontend.md`'s "Passing actions down")
/// used directly as `on-click={<propName>}` on the component's OWN element does not wire up on the
/// lowered VM tier (the tier a real browser runs): `lowered_vm_tier_lock` and `largo test --tiers
/// all` both show it stopping at the very first click with `Nothing answers a click on "#…"`, while
/// the tree-walker runs it correctly. Found via `examples/modal_toast_keyboard`'s `Modal (close:
/// fn() -> Unit)`; reduced here to the smallest shape docs/frontend.md itself teaches.
#[test]
fn a_callback_props_on_click_wires_up_on_the_lowered_vm_tier() {
    const CALLBACK_PROP_ON_CLICK: &str = r##"
## Component Child (onGo: fn() -> Unit)

View:
    <button id="go" on-click={onGo}>go</button>

## Page /

State:
    Let count be 0.

Actions:
    To bump:
        Set count to count + 1.

View:
    <Child onGo={bump}/>
    <p id="count">{count}</p>

## Test: clicking the child calls the parent's callback prop

Click "#go".
Expect that the text of "#count" is equal to "1".
"##;
    let outcome = one_outcome(CALLBACK_PROP_ON_CLICK);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "a callback prop (`fn() -> Unit`) used directly as `on-click={{propName}}` on the \
         component's own element — exactly `docs/frontend.md`'s \"Passing actions down\" pattern \
         — must wire up on every tier, not only the tree-walker: {:?}",
        outcome.failures
    );
}

/// G16 — the shared cross-tier corpus discovery (`lowered_vm_tier_lock`'s `example_pages()`, and
/// likely `browser_tier_lock`'s sibling) reads each `examples/*/pages/*.lg` file as an ISOLATED
/// string via `page_host::compare_tests`, which has no file-loader and cannot resolve `Import` —
/// unlike `largo check`/`largo test`/`examples_lock.rs`, all of which assemble the project's full
/// closure first. A page written exactly to `docs/frontend.md`'s Layouts section (`Import
/// Layout.` + `{extends Site}`, `## Component Site` in a sibling `src/layout.lg`) — verified
/// working on every real product surface (`largo check`, `largo test`, `--tiers all`, `--browser`,
/// `largo build --static`, and the resulting `dist/*/index.html`) — fails this ONE discovery path,
/// with a message that blames the `{extends}` target for not existing, when the real fact (the
/// import was never resolved because this discovery path has no loader at all) is invisible.
#[test]
fn a_page_using_import_and_extends_is_not_misdiagnosed_as_missing_its_layout() {
    use logicaffeine_compile::page_host::{compare_tests, PageTierKind};

    // The exact shape `docs/frontend.md`'s Layouts section teaches, and `examples/multi_page_site`
    // uses verbatim.
    let unassembled = "\
Import Layout.

## Page /about

View:
    {extends Site}
    {block content:}<h1 id=\"title\">About</h1>{end}
";
    let err = compare_tests(unassembled, &[PageTierKind::LoweredVm], 0, 50_000_000, None)
        .expect_err(
            "compare_tests has no loader and cannot resolve `Import Layout.` from a bare string",
        );
    assert!(
        err.contains("Import") || err.contains("import") || err.contains("layout"),
        "a page whose ONLY problem is an unresolved `Import` must be told that — not left to fail \
         several lines later at `{{extends}}` with a message that blames the layout component for \
         not existing, when it exists exactly where the docs say to put it: {err:?}"
    );

    // The identical page, with the layout's own definition inlined instead of imported, is
    // everything `compare_tests` needs and must succeed — proving the fix belongs in the CALLER
    // (assemble the project closure before calling `compare_tests`, the way `examples_lock.rs`
    // already does), not in the language, the docs, or `compare_tests` itself.
    let assembled = "\
## Component Site

View:
    <header>nav</header>
    <main>{block content}</main>

## Page /about

View:
    {extends Site}
    {block content:}<h1 id=\"title\">About</h1>{end}

## Test: it renders under the layout

Expect that the text of \"#title\" is equal to \"About\".
";
    let ok = compare_tests(assembled, &[PageTierKind::LoweredVm], 0, 50_000_000, None);
    assert!(
        ok.is_ok(),
        "the SAME layout, inlined rather than imported, must compare cleanly: {ok:?}"
    );
}

/// G12 — `routePath()` never reflects a non-root `## Page`'s own URL: nothing calls
/// `crate::semantics::dom::init_route` (zero callers anywhere in `logicaffeine_compile`, checked by
/// grep) to seed a mounted page's own declared path, so `routePath()` reads the boot default `"/"`
/// on every tier this lane could reach — reproduced live: `largo build --static`'s pre-rendered
/// `dist/about/index.html` marks the Home nav link active, not itself, exactly matching what this
/// test pins at the API level. `class:active={routePath() is equal to "/about"}` — the idiomatic,
/// documented way to mark a nav's current link (`docs/frontend.md`'s Layouts section; `routePath` is
/// its own documented Navigation-reference function) — cannot be written correctly for any page but
/// the root.
#[test]
fn a_pages_own_test_sees_its_own_declared_route() {
    const NON_ROOT_PAGE: &str = r##"
## Page /about

View:
    <p id="path">{routePath()}</p>

## Test: the page's own test sees its own URL

Expect that the text of "#path" is equal to "/about".
"##;
    let outcome = one_outcome(NON_ROOT_PAGE);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "a `## Page /about`'s own auto-mounted test must see `routePath()` equal to \"/about\" — \
         the page's own declared URL — not the boot default \"/\": {:?}",
        outcome.failures
    );
}

/// G8 — the element-state page questions (`"#sel" is visible`/`hidden`/`disabled`/`checked`/
/// `focused` — `crates/logicaffeine_language/src/parser/mod.rs`'s element-state block) have no
/// negated form, unlike `x is none`, whose `is not none` is handled two blocks earlier in the SAME
/// function. `"#sel" is not disabled` instead falls into the general `is not <expr>` path, which
/// parses `disabled` as an ordinary identifier — `largo check` reports `Check passed` — and fails
/// only when the test actually runs, with "Undefined variable: disabled": neither a parse-time
/// refusal naming the missing negated form, nor anything docs/frontend.md's "Testing a page"
/// section (which lists only the positive forms) warns a reader away from.
#[test]
fn a_disabled_page_question_negates_cleanly() {
    const NOT_DISABLED: &str = r##"
## Page /

State:
    Let on be false.

View:
    <button id="btn" disabled={on}>go</button>

## Test: the button is not disabled by default

Expect that "#btn" is not disabled.
"##;
    let outcome = one_outcome(NOT_DISABLED);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "`\"#btn\" is not disabled` is the natural negation of the documented `\"#sel\" is \
         disabled` page question and the button in this fixture genuinely is not disabled — the \
         test must pass, not fail with an \"Undefined variable: disabled\" that treats the \
         adjective as an unbound identifier: {:?}",
        outcome.failures
    );
}

/// G1 — `largo test`'s cross-tier safety net (`Ast` vs the lowered bytecode VM, and, under
/// `--browser`, a real headless Chrome) is gated on `TestOutcome::mounted`, which
/// `crates/logicaffeine_compile/src/testrun.rs` computes as "this test's body contains a literal
/// `Mount` statement" — never as "this test drives a page that is, in fact, mounted." Every page
/// test `docs/frontend.md`, `LOGOS_QUICKGUIDE.md`, `AGENTS.md`'s template and this gauntlet's own
/// examples teach — an ordinary `## Page` test with no `Mount` statement, because the page "mounts
/// itself" — never sets `mounted`, so the entire cross-tier check this campaign built `largo test
/// --browser` for never runs against it.
///
/// The divergence these two tests hand the check exists BY CONSTRUCTION: the candidate tier is
/// `common/divergent_page.rs`'s, which replays the reference tier's own trace and loses the effect
/// of the `Click`. A real miscompile would be a fixture that stops diverging the day it is fixed —
/// these once stood on `Save` of a list, and went red when G2 was fixed. This one cannot close, so
/// what the tests hold is exactly G1: the `mounted` gate, and the check it opens.
const PAGE_LEVEL_FIXTURE: &str = r##"
## Page /

State:
    Let n be 0.

Actions:
    To bump:
        Set n to n + 1.

View:
    <button id="add" on-click={bump}>add</button>
    <p id="count">{n}</p>

## Test: an ordinary page test drives a page that mounts itself

Click "#add".
Expect that the text of "#count" is equal to "1".
"##;

/// The SAME fixture, with the one change the docs say is reserved for testing a bare `## Component`
/// on its own (frontend.md: "the one place `Mount` belongs in a page file") — an explicit `Mount`,
/// the form that set `mounted` before G1 was fixed. It is the proof that the check itself works, so
/// that the page-level test below is about the gate alone.
const MOUNT_LEVEL_FIXTURE: &str = r##"
## Component Counter

State:
    Let n be 0.

Actions:
    To bump:
        Set n to n + 1.

View:
    <button id="add" on-click={bump}>add</button>
    <p id="count">{n}</p>

## Page /
View:
    <Counter/>

## Test: an explicitly mounted component test drives the same page

Mount Counter.
Click "#add".
Expect that the text of "#count" is equal to "1".
"##;

fn one_outcome(source: &str) -> logicaffeine_compile::testrun::TestOutcome {
    let report = run_tests(source, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the fixture must parse and run: {e}"));
    assert_eq!(
        report.outcomes.len(),
        1,
        "the fixture declares exactly one ## Test"
    );
    report.outcomes.into_iter().next().unwrap()
}

/// `source`'s one test under `largo test`'s own runner, checked against the lowered VM AND a tier
/// that loses the effect of statement `lost` — what the check saw, and what it handed that tier.
fn outcome_against_a_divergent_tier(
    source: &str,
    lost: usize,
) -> (
    logicaffeine_compile::testrun::TestOutcome,
    divergent_page::Seen,
) {
    use logicaffeine_compile::page_host::PageTierKind;
    let tier = divergent_page::LosesOneStatement::at(source, lost);
    let config = TestConfig {
        page_tiers: vec![
            PageTierKind::LoweredHere,
            PageTierKind::LoweredVm,
            PageTierKind::Browser,
        ],
        ..TestConfig::default()
    };
    let report =
        logicaffeine_compile::testrun::run_tests_with_launcher(source, &config, Some(&tier))
            .unwrap_or_else(|e| panic!("the fixture must parse and run: {e}"));
    assert_eq!(
        report.outcomes.len(),
        1,
        "the fixture declares exactly one ## Test"
    );
    let seen = std::mem::take(&mut *tier.seen().borrow_mut());
    (report.outcomes.into_iter().next().unwrap(), seen)
}

/// The check must name the lost statement and nothing else: one failure, from the divergent tier
/// alone (the lowered VM, checked first, agreed), first differing at exactly `lost`.
fn assert_caught_at(outcome: &logicaffeine_compile::testrun::TestOutcome, lost: usize) {
    use logicaffeine_compile::testrun::TierEvidence;
    assert_eq!(
        outcome.verdict,
        Verdict::Failed,
        "a divergence fails the test: {:?}",
        outcome.failures
    );
    match &outcome.tier {
        Some(TierEvidence::Diverged {
            statement, detail, ..
        }) => {
            assert_eq!(
                *statement,
                Some(lost),
                "the first differing statement is the lost one: {detail}"
            );
            assert!(
                detail.contains(&format!("document differs after statement {lost}")),
                "the report names the axis and the statement: {detail}"
            );
        }
        other => panic!(
            "expected the divergence to be caught, got {other:?}: {:?}",
            outcome.failures
        ),
    }
    assert_eq!(
        outcome.failures.len(),
        1,
        "the divergence is the only failure: {:?}",
        outcome.failures
    );
    assert!(
        outcome.failures[0].starts_with("TIER DIVERGENCE on browser:"),
        "reported against the tier that diverged, after the lowered VM agreed: {:?}",
        outcome.failures
    );
}

#[test]
fn a_mounted_test_already_catches_a_divergent_tier() {
    let (outcome, seen) = outcome_against_a_divergent_tier(MOUNT_LEVEL_FIXTURE, 2);
    assert!(
        outcome.mounted,
        "an explicit `Mount` sets TestOutcome::mounted"
    );
    assert_caught_at(&outcome, 2);
    assert_eq!(
        seen.launches,
        ["/"],
        "the check launched the page once, at its route"
    );
    assert!(
        seen.drives.iter().any(|d| matches!(
            d,
            logicaffeine_compile::page_host::Drive::Click { selector } if selector == "#add"
        )),
        "the check drove the click through the candidate tier: {:?}",
        seen.drives
    );
    assert_eq!(
        one_outcome(MOUNT_LEVEL_FIXTURE).verdict,
        Verdict::Passed,
        "the fixture itself passes and agrees on the default tiers — the divergence is the \
         candidate tier's alone"
    );
}

#[test]
fn an_ordinary_page_test_gets_the_same_cross_tier_check_a_mounted_one_does() {
    let (outcome, seen) = outcome_against_a_divergent_tier(PAGE_LEVEL_FIXTURE, 1);
    assert!(
        outcome.mounted,
        "a `## Page`'s own auto-mounted test drives a mounted page exactly as much as an explicit \
         `Mount` does — `TestOutcome::mounted` must say so, not just report the literal presence of \
         the word `Mount` in the test body (testrun.rs's `mounted` computation)"
    );
    assert_caught_at(&outcome, 1);
    assert_eq!(
        seen.launches,
        ["/"],
        "the check launched the page once, at its own route"
    );
    assert_eq!(
        seen.drives,
        [logicaffeine_compile::page_host::Drive::Click {
            selector: "#add".into()
        }],
        "the page mounted itself on arrival, and the one driving statement was handed over"
    );
    assert_eq!(
        one_outcome(PAGE_LEVEL_FIXTURE).verdict,
        Verdict::Passed,
        "the fixture itself passes and agrees on the default tiers — the divergence is the \
         candidate tier's alone"
    );
}

/// G2/G3 — `Save`/`Load` on a field whose type is not Text/Int/Real/Bool (a `Seq of Text` here,
/// standing in for the `Seq of Todo` a real todo app needs to persist) once silently defaulted to
/// the `Text` tag (`Save`, miscompiling) or silently dropped the containing action from the
/// compiled program (`Load`). Both now go through the derived JSON codec, so the list must make the
/// whole trip — written by `Save`, read back by `Load` — and come back the same list on every tier:
/// the tree-walker, the lowered bytecode VM (`largo test`'s own cross-tier check), and the page's
/// real WebAssembly module, run against a host whose document and storage ARE the in-process tiers'
/// own (`semantics::dom`, `semantics::storage`), so the text the browser tier stores is compared
/// byte for byte with the text the lowered VM stores, and each tier loads what the other saved.
mod list_and_variant_lowering_bugs {
    use logicaffeine_compile::compile::compile_to_wasm;
    use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
    use logicaffeine_compile::testrun::{run_tests, TestConfig, TierEvidence, Verdict};

    const SAVE_A_LIST_FIELD: &str = "\
## Page /

State:
    Let names be a new Seq of Text.

Actions:
    To addName:
        Push \"ada\" to names.
        Save names as \"names\".
    To forgetAndReload:
        Set names to a new Seq of Text.
        Load \"names\" into names.

View:
    <button id=\"add\" on-click={addName}>add</button>
    <button id=\"reload\" on-click={forgetAndReload}>reload</button>
    <p id=\"count\">{length of names}</p>
    <ul id=\"names\">{for each n in names:}<li>{n}</li>{end}</ul>
";

    const SAVE_A_LIST_FIELD_TEST: &str = "
## Test: a saved list survives being cleared and loaded back

Click \"#add\".
Expect that the text of \"#count\" is equal to \"1\".
Click \"#reload\".
Expect that the text of \"#count\" is equal to \"1\".
Expect that the text of \"#names\" is equal to \"ada\".
";

    const LOAD_INTO_A_LIST_FIELD: &str = "\
## Page /

State:
    Let names be a new Seq of Text.

Actions:
    To seed:
        Push \"ada\" to names.
        Push \"bob\" to names.
        Save names as \"names\".
        Set names to a new Seq of Text.
    To load:
        Load \"names\" into names.

View:
    <button id=\"seed\" on-click={seed}>seed</button>
    <button id=\"load\" on-click={load}>load</button>
    <p id=\"count\">{length of names}</p>
    <ul id=\"names\">{for each n in names:}<li>{n}</li>{end}</ul>
";

    const LOAD_INTO_A_LIST_FIELD_TEST: &str = "
## Test: the list loaded back from storage is the list that was saved

Click \"#seed\".
Expect that the text of \"#count\" is equal to \"0\".
Click \"#load\".
Expect that the text of \"#count\" is equal to \"2\".
Expect that the text of \"#names\" is equal to \"adabob\".
";

    /// The page's own `## Test`, through `largo test`'s runner with its default page tiers: the
    /// reference tier passes every `Expect`, and the lowered VM traces the run identically.
    fn passes_and_agrees(page: &str, test: &str) {
        let source = format!("{page}{test}");
        let report = run_tests(&source, &TestConfig::default())
            .unwrap_or_else(|e| panic!("the fixture must parse: {e}\n{source}"));
        assert_eq!(report.outcomes.len(), 1, "one ## Test:\n{source}");
        let o = &report.outcomes[0];
        assert!(o.mounted, "a `## Page`'s own test drives a mounted page");
        assert_eq!(o.verdict, Verdict::Passed, "{:?}\n{source}", o.failures);
        assert_eq!(
            o.tier,
            Some(TierEvidence::Agreed),
            "the lowered VM must trace the round trip exactly as the tree-walker does:\n{source}"
        );
    }

    /// What the lowered VM had stored, `(area, key, value)`, after statement `after` of the test.
    fn lowered_vm_storage_after(
        page: &str,
        test: &str,
        after: usize,
    ) -> Vec<(String, String, String)> {
        let source = format!("{page}{test}");
        let all = compare_tests(&source, &[PageTierKind::LoweredVm], 0, 50_000_000, None)
            .unwrap_or_else(|e| panic!("the fixture must parse: {e}\n{source}"));
        assert_eq!(all.len(), 1, "one page test:\n{source}");
        match &all[0].candidates[0].1 {
            TierRun::Traced(t) => t
                .steps
                .iter()
                .find(|s| s.statement == after)
                .unwrap_or_else(|| panic!("no step {after} in {:?}", t.steps))
                .snapshot
                .storage
                .clone(),
            TierRun::Refused { statement, reason } => {
                panic!("the lowered VM refused statement {statement}: {reason}\n{source}")
            }
        }
    }

    /// `Save` of a list field writes the list, and `Load` of the same key brings the SAME list
    /// back — its length and its element — on the tree-walker, on the lowered VM, and in the
    /// browser, where the stored text is the lowered VM's to the byte.
    #[test]
    fn saving_a_list_field_round_trips_through_load_on_every_tier() {
        passes_and_agrees(SAVE_A_LIST_FIELD, SAVE_A_LIST_FIELD_TEST);
        let vm_saved = lowered_vm_storage_after(SAVE_A_LIST_FIELD, SAVE_A_LIST_FIELD_TEST, 1);
        assert!(
            vm_saved.iter().any(|(_, key, _)| key == "names"),
            "the lowered VM's `Save` writes the key it names: {vm_saved:?}"
        );

        #[cfg(feature = "wasm-jit")]
        {
            let mut page = wasm::Page::load(SAVE_A_LIST_FIELD, &[]);
            assert_eq!(page.text("#count"), "0", "the page arrives empty");
            page.click("#add");
            assert_eq!(page.text("#count"), "1", "the push repaints in the browser");
            assert_eq!(
                page.storage(),
                vm_saved,
                "the browser stores the list as exactly the text the lowered VM stores"
            );
            page.click("#reload");
            assert_eq!(
                page.text("#count"),
                "1",
                "the cleared list is loaded back whole"
            );
            assert_eq!(
                page.text("#names"),
                "ada",
                "and it is the list that was saved"
            );
        }
    }

    /// G5 — an `Inspect` arm naming a variant that does not belong to the matched value's type
    /// (`When Err (message):` against a `JsonResult`, whose variants are `JOk`/`JErr`, not
    /// `Ok`/`Err`) is accepted at compile time — `Check passed`, byte for byte, on
    /// `logicaffeine-cli`'s own `--json` check — and fails only at runtime, with a message that
    /// names neither the mismatched variant nor `JsonResult` nor a location:
    /// "Inspect has no arm for the value and no Otherwise (matches must be exhaustive)"
    /// (`interpreter.rs::inspect_unhandled`). A compiler that types `Inspect` arms at all should
    /// refuse this the moment the arm is written, exactly as it would refuse a field access on a
    /// name no variant of the type carries.
    #[test]
    fn an_inspect_arm_naming_the_wrong_variant_is_refused_at_compile_time() {
        use logicaffeine_compile::compile::compile_to_program;

        const WRONG_VARIANT_NAME: &str = "\
## To classify (raw: Text) -> Int:
    Inspect jsonTryParse(raw):
        When JOk (doc):
            Return 1.
        When Err (message):
            Return 2.
    Return 0.

## Main
Show classify(\"\").
";
        let result = compile_to_program(WRONG_VARIANT_NAME);
        assert!(
            result.is_err(),
            "`When Err (message):` matches nothing `jsonTryParse`'s `JsonResult` can produce \
             (its variants are `JOk`/`JErr`) — this must be refused at compile time, naming the \
             bad arm and the type it does not belong to, not accepted and left to fail at runtime \
             with a message that names neither."
        );
    }

    /// G5 REGRESSION — `code_examples_all_tiers` (release-prep) caught this exact shape breaking
    /// right after the fix above landed: `examples/code/types/option-result.logos`'s own
    /// `Outcome`/`Success`/`Failure`, where `Failure` is ALSO a variant the stdlib `Severity`
    /// enum (`assets/std/io.lg`) declares. Before `resolve_arm_variant` existed,
    /// `TypeRegistry::find_variant`'s own `HashMap` walk could resolve the Inspect's SECOND arm
    /// (`Failure`) to `Severity` even though its FIRST arm (`Success`, which only `Outcome`
    /// declares) had already committed the whole Inspect to `Outcome` — refusing the program
    /// outright (`'Failure' is a variant of 'Severity', not 'Outcome'`). Fixed by resolving each
    /// arm against the enum the Inspect has already established FIRST
    /// (`parser/mod.rs::resolve_arm_variant`), before ever falling back to the registry's own
    /// unordered global search. The full pipeline is exercised (not just `compile_to_program`'s
    /// `Ok`/`Err`): the exact expected OUTPUT, on both tiers, since a wrong resolution could also
    /// silently print the wrong thing rather than refuse.
    #[test]
    fn an_inspect_established_by_its_first_arm_is_not_shadowed_by_a_stdlib_variant_of_the_same_name(
    ) {
        use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

        const OPTION_RESULT_SHAPE: &str = "\
## Definition
An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## To checked_divide (a: Int) and (b: Int) -> Outcome:
    If b equals 0:
        Return a new Failure with message \"cannot divide {a} by zero\".
    Return a new Success with value (a / b).

## Main

Let good be checked_divide(10, 2).
Inspect good:
    When Success (v): Show \"10 / 2 = {v}\".
    When Failure (m): Show \"error: {m}\".

Let bad be checked_divide(5, 0).
Inspect bad:
    When Success (v): Show \"5 / 0 = {v}\".
    When Failure (m): Show \"error: {m}\".
";
        let tw = tw_outcome(OPTION_RESULT_SHAPE);
        assert_eq!(
            tw.error, None,
            "the reference tier must run this cleanly: {:?}",
            tw.error
        );
        assert_eq!(
            tw.output.trim(),
            "10 / 2 = 5\nerror: cannot divide 5 by zero"
        );
        let vm = vm_outcome(OPTION_RESULT_SHAPE);
        assert_eq!(
            (vm.output.trim(), vm.error.as_deref()),
            (tw.output.trim(), tw.error.as_deref()),
            "the VM must agree with the reference tier"
        );
    }

    /// G6 — a derived field (`Let visible always be filteredTodos(todos, filter).`) whose function
    /// returns `Seq of <record>` loses that record's field layout on the WASM AOT backend: `t's
    /// id` inside a loop over the DERIVED field crashes with "field access with no static slot",
    /// while the identical loop over the PLAIN state field `todos` (same declared type `Seq of
    /// Todo`) compiles and runs. Bisected in `/tmp/repro_slot` (not committed — reduced from
    /// `examples/todo`): every other change tried in isolation (`remove`, `nextIdFor`, the filter
    /// buttons, an `{else:}` arm) left `largo check` green; only switching the loop's source from
    /// `todos` to the derived `visible` field introduced the failure.
    #[test]
    fn a_derived_list_field_from_a_function_keeps_its_element_type_on_the_wasm_backend() {
        use logicaffeine_compile::compile::compile_to_wasm;

        const DERIVED_LIST_FIELD: &str = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## To keepAll (todos: Seq of Todo) -> Seq of Todo:
    Let mutable out be a new Seq of Todo.
    Repeat for t in todos:
        Push t to out.
    Return out.

## Page /

State:
    Let todos be a new Seq of Todo.
    Let visible always be keepAll(todos).

Actions:
    To add:
        Push (a new Todo with id 1 and text \"milk\") to todos.

View:
    <button id=\"add\" on-click={add}>add</button>
    <ul>
        {for each t in visible, keyed by t's id:}
            <li id=\"row-{t's id}\">{t's text}</li>
        {end}
    </ul>
";
        let result = compile_to_wasm(DERIVED_LIST_FIELD);
        assert!(
            result.is_ok(),
            "looping over a derived `Seq of <record>` field (built by a function returning that \
             same declared type) and reading a field of its loop variable must compile for the \
             browser exactly as looping over a plain, non-derived state field of the identical \
             type already does: {:?}",
            result.err()
        );
    }

    /// G10 — `docs/frontend.md`'s Fetching section teaches `e's message` as how a page explains WHY
    /// a typed fetch failed ("A status outside 200-299 ... are all `Err`, with `e's message` saying
    /// which"). Reading it crashes the WASM AOT backend with "field access with no static slot" —
    /// reproduced with an `Ok` type as plain as `Int`, so it is not specific to a record or list
    /// `Ok` payload; the tree-walker and the lowered bytecode VM both handle it correctly.
    #[test]
    fn reading_a_typed_fetch_errors_message_compiles_for_the_browser() {
        let source = "\
## Page /

State:
    Let lastError be \"\".
    Let count be 0.

Actions:
    To load:
        Get \"/api/count\" as Int, then loaded.

    To loaded (answer: Result of Int and HttpError):
        Inspect answer:
            When Ok (n):
                Set count to n.
            When Err (e):
                Set lastError to e's message.

View:
    <button id=\"load\" on-click={load}>load</button>
    <p id=\"count\">{count}</p>
    <p id=\"err\">{lastError}</p>
";
        let result = compile_to_wasm(source);
        assert!(
            result.is_ok(),
            "`e's message` on a typed fetch's `Err (e)` binding is the documented way to explain a \
             failed request and must compile for the browser exactly as it already runs correctly \
             on the tree-walker and the lowered bytecode VM: {:?}",
            result.err()
        );
    }

    /// `Load` into a list field is never dropped from the build: the action holding it runs and
    /// brings back the whole saved list, in order, on every tier — and a page reloaded in the
    /// browser loads the list the LOWERED VM saved, so one tier's stored text is the other's
    /// loadable text.
    #[test]
    fn loading_into_a_list_field_runs_on_every_tier() {
        passes_and_agrees(LOAD_INTO_A_LIST_FIELD, LOAD_INTO_A_LIST_FIELD_TEST);
        let vm_saved =
            lowered_vm_storage_after(LOAD_INTO_A_LIST_FIELD, LOAD_INTO_A_LIST_FIELD_TEST, 1);
        assert!(
            vm_saved.iter().any(|(_, key, _)| key == "names"),
            "the lowered VM's `Save` writes the key it names: {vm_saved:?}"
        );

        #[cfg(feature = "wasm-jit")]
        {
            let mut page = wasm::Page::load(LOAD_INTO_A_LIST_FIELD, &[]);
            page.click("#seed");
            assert_eq!(
                page.text("#count"),
                "0",
                "the seeded list is saved, then cleared"
            );
            assert_eq!(
                page.storage(),
                vm_saved,
                "the browser saves what the lowered VM saves"
            );
            page.click("#load");
            assert_eq!(
                page.text("#count"),
                "2",
                "the `load` action is in the build and runs"
            );
            assert_eq!(
                page.text("#names"),
                "adabob",
                "the list comes back whole and in order"
            );

            let mut reloaded = wasm::Page::load(LOAD_INTO_A_LIST_FIELD, &vm_saved);
            assert_eq!(
                reloaded.text("#count"),
                "0",
                "a reloaded page starts from its initializer"
            );
            reloaded.click("#load");
            assert_eq!(
                reloaded.text("#count"),
                "2",
                "the list the lowered VM saved survives the reload"
            );
            assert_eq!(reloaded.text("#names"), "adabob");
        }
    }

    /// A page's real WebAssembly module, hosted on the in-process tiers' own document and storage.
    ///
    /// Every `dom_*`/`storage_*` import is answered by `semantics::dom`/`semantics::storage` — the
    /// recorder the tree-walker and the lowered VM paint into — so the browser tier is read through
    /// the very queries a `## Test` reads, and what it stores is the text the other tiers load. A
    /// page owns both thread-locals for as long as it is being driven.
    #[cfg(feature = "wasm-jit")]
    mod wasm {
        use super::super::common::wasm_host::{
            link_failure_hooks, link_native_stream, link_value_imports, read_text,
        };
        use logicaffeine_compile::semantics::{dom, patch, storage};
        use wasmi::{Caller, Engine, Instance, Linker, Module, Store};

        /// The page's host keeps its document, its listeners and its storage in `semantics::dom`
        /// and `semantics::storage`; the store carries nothing of its own.
        type Host = ();
        type C<'a> = Caller<'a, Host>;

        pub struct Page {
            store: Store<Host>,
            inst: Instance,
        }

        /// Answer host text through the module's own allocator, as `runtime.js`'s `allocText` does.
        fn host_text(c: &mut C<'_>, s: &str) -> i32 {
            let new_text = c
                .get_export("__logos_text_new")
                .and_then(|e| e.into_func())
                .expect("a module reading host text exports __logos_text_new")
                .typed::<i32, i32>(&*c)
                .expect("__logos_text_new: i32 -> i32");
            let handle = new_text
                .call(&mut *c, s.len() as i32)
                .expect("__logos_text_new does not trap");
            let mem = c
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("memory");
            let ptr = {
                let d = mem.data(&*c);
                i32::from_le_bytes(
                    d[handle as usize + 8..handle as usize + 12]
                        .try_into()
                        .expect("4 bytes"),
                )
            };
            mem.write(&mut *c, ptr as usize, s.as_bytes())
                .expect("the block holds the text");
            handle
        }

        /// The bytes of the `Seq of Int` patch buffer at `handle`: `[len][cap][data_ptr]`, 8-byte slots.
        fn patch_bytes(c: &C<'_>, handle: i32) -> Vec<u8> {
            let mem = c
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("memory");
            let d = mem.data(c);
            let at = handle as usize;
            let len = i32::from_le_bytes(d[at..at + 4].try_into().expect("4 bytes")) as usize;
            let ptr = i32::from_le_bytes(d[at + 8..at + 12].try_into().expect("4 bytes")) as usize;
            (0..len)
                .map(|i| {
                    i64::from_le_bytes(d[ptr + i * 8..ptr + i * 8 + 8].try_into().expect("8 bytes"))
                        as u8
                })
                .collect()
        }

        fn link(l: &mut Linker<Host>) {
            link_value_imports::<Host>(l);
            link_failure_hooks::<Host>(l);
            l.func_wrap("env", "dom_body", |_c: C<'_>| -> i64 { dom::body() })
                .unwrap();
            l.func_wrap("env", "dom_create_element", |c: C<'_>, t: i32| -> i64 {
                dom::create_element(&read_text(&c, t))
            })
            .unwrap();
            l.func_wrap("env", "dom_create_text_node", |c: C<'_>, t: i32| -> i64 {
                dom::create_text_node(&read_text(&c, t))
            })
            .unwrap();
            l.func_wrap("env", "dom_set_text", |c: C<'_>, n: i64, t: i32| {
                dom::set_text(n, &read_text(&c, t))
            })
            .unwrap();
            l.func_wrap("env", "dom_set_html", |c: C<'_>, n: i64, t: i32| {
                dom::set_html(n, &read_text(&c, t))
            })
            .unwrap();
            l.func_wrap("env", "dom_clear_children", |_c: C<'_>, n: i64| {
                dom::clear_children(n)
            })
            .unwrap();
            l.func_wrap(
                "env",
                "dom_set_attribute",
                |c: C<'_>, n: i64, k: i32, v: i32| {
                    dom::set_attribute(n, &read_text(&c, k), &read_text(&c, v))
                },
            )
            .unwrap();
            l.func_wrap("env", "dom_append_child", |_c: C<'_>, p: i64, ch: i64| {
                dom::append_child(p, ch)
            })
            .unwrap();
            l.func_wrap("env", "dom_get_text", |mut c: C<'_>, n: i64| -> i32 {
                let t = dom::get_text(n);
                host_text(&mut c, &t)
            })
            .unwrap();
            l.func_wrap("env", "dom_query_selector", |c: C<'_>, s: i32| -> i64 {
                dom::query_selector(&read_text(&c, s))
            })
            .unwrap();
            crate::common::wasm_host::link_native_attributes::<Host>(l);
            l.func_wrap(
                "env",
                "dom_listen_flags",
                |c: C<'_>, n: i64, e: i32, flags: i64| {
                    dom::listen(n, &read_text(&c, e), flags as u8)
                },
            )
            .unwrap();
            l.func_wrap("env", "dom_apply", |c: C<'_>, patches: i32| {
                let ops = patch::decode(&patch_bytes(&c, patches)).unwrap_or_else(|e| {
                    panic!("the module emitted an undecodable patch stream: {e}")
                });
                patch::apply(&ops).unwrap_or_else(|e| {
                    panic!("the module emitted a patch stream the host cannot honour: {e}")
                });
            })
            .unwrap();
            link_native_stream::<Host>(l);
            l.func_wrap(
                "env",
                "dom_note_turn",
                |_c: C<'_>, _name: i32, _key: i32, _changes: i32, _failure: i32| {},
            )
            .unwrap();
            l.func_wrap("env", "dom_journal_detail", |_c: C<'_>| -> i64 { 0 })
                .unwrap();
            l.func_wrap("env", "dom_wake_after", |_c: C<'_>, _ms: i64| {})
                .unwrap();
            l.func_wrap("env", "storage_set", |c: C<'_>, a: i32, k: i32, v: i32| {
                storage::set(&read_text(&c, a), &read_text(&c, k), &read_text(&c, v))
                    .unwrap_or_else(|e| panic!("storage_set: {e}"));
            })
            .unwrap();
            l.func_wrap(
                "env",
                "storage_get",
                |mut c: C<'_>, a: i32, k: i32| -> i32 {
                    let found = storage::get(&read_text(&c, a), &read_text(&c, k))
                        .unwrap_or_else(|e| panic!("storage_get: {e}"));
                    match found {
                        Some(v) => host_text(&mut c, &v),
                        None => 0,
                    }
                },
            )
            .unwrap();
            l.func_wrap("env", "storage_remove", |c: C<'_>, a: i32, k: i32| {
                storage::remove(&read_text(&c, a), &read_text(&c, k))
                    .unwrap_or_else(|e| panic!("storage_remove: {e}"));
            })
            .unwrap();
        }

        impl Page {
            /// A visitor arriving at `source`'s page in a browser whose storage already holds
            /// `stored` (`(area, key, value)` rows): a fresh document, the module instantiated, and
            /// its `main` — the page's own mount — run.
            pub fn load(source: &str, stored: &[(String, String, String)]) -> Page {
                let module =
                    logicaffeine_compile::compile::compile_to_wasm(source).unwrap_or_else(|e| {
                        panic!("the page must compile for the browser: {e:?}\n{source}")
                    });
                dom::reset();
                storage::reset();
                for (area, key, value) in stored {
                    storage::set(area, key, value)
                        .unwrap_or_else(|e| panic!("seeding storage: {e}"));
                }
                let engine = Engine::default();
                let m =
                    Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("valid wasm: {e}"));
                let mut l = Linker::<Host>::new(&engine);
                link(&mut l);
                crate::common::wasm_host::link_unmodelled_imports(&mut l);
                let mut store = Store::new(&engine, ());
                let inst = l
                    .instantiate(&mut store, &m)
                    .unwrap_or_else(|e| panic!("the page must instantiate: {e}"))
                    .start(&mut store)
                    .unwrap_or_else(|e| panic!("the start function must not trap: {e}"));
                inst.get_typed_func::<(), ()>(&store, "main")
                    .expect("the page exports main")
                    .call(&mut store, ())
                    .unwrap_or_else(|e| panic!("the page's mount must not trap: {e}"));
                Page { store, inst }
            }

            /// Click the element `selector` names: hand the click to the page's node dispatcher, as
            /// `runtime.js` hands a browser's click over.
            pub fn click(&mut self, selector: &str) {
                let node = dom::query_selector(selector);
                assert!(node >= 0, "nothing matches {selector}: {}", dom::render());
                let answered = crate::common::wasm_host::dispatch_node(
                    &mut self.store,
                    &self.inst,
                    node,
                    "click",
                    "",
                )
                .unwrap_or_else(|e| panic!("clicking {selector} must not trap: {e}"));
                assert!(
                    answered,
                    "{selector} has no click listener: {}",
                    dom::render()
                );
            }

            /// The text of the element `selector` names, as a `## Test` reads it.
            pub fn text(&self, selector: &str) -> String {
                let node = dom::query_selector(selector);
                assert!(node >= 0, "nothing matches {selector}: {}", dom::render());
                dom::text_content(node)
            }

            /// Every stored row, `(area, key, value)`, in the order a page snapshot lists them.
            pub fn storage(&self) -> Vec<(String, String, String)> {
                let mut rows = Vec::new();
                for area in [storage::LOCAL, storage::SESSION] {
                    for key in storage::keys(area).unwrap_or_default() {
                        let value = storage::get(area, &key).ok().flatten().unwrap_or_default();
                        rows.push((area.to_string(), key, value));
                    }
                }
                rows
            }
        }
    }
}

/// G18 (gauntlet2) — a `## Store` cannot hold a `Seq` field AT ALL, in ANY mode, even a plain
/// in-memory `## Store Name` with no `persisted as`/`preserved` lifetime. `docs/frontend.md`'s
/// "Shared state: stores" section states the Text/Int/Real/Bool restriction only for a store that
/// keeps its state between page loads ("A kept store holds Text, Int, Real and Bool fields.") —
/// nothing says an EPHEMERAL, single-page-load store hits the identical restriction. Building
/// `examples/shopping_cart`'s natural first draft (`## Store Cart` with `Let items be a new Seq of
/// CartItem.`, no persistence at all) hits this immediately: `largo check` refuses the whole page,
/// naming the store, not a field or a lifetime.
#[test]
fn a_local_stores_seq_field_compiles_for_the_browser() {
    use logicaffeine_compile::compile::compile_to_wasm;

    const STORE_WITH_A_LIST: &str = "\
## Store Tags

State:
    Let names be a new Seq of Text.

Actions:
    To addName (n: Text):
        Push n to names.

## Component Badge

Use Tags.

View:
    <span id=\"count\">{length of Tags's names}</span>

## Page /

Use Tags.

Actions:
    To add:
        Call Tags's addName with \"x\".

View:
    <Badge/>
    <button id=\"add\" on-click={add}>add</button>
";
    let result = compile_to_wasm(STORE_WITH_A_LIST);
    assert!(
        result.is_ok(),
        "a `## Store`'s shared state must be able to hold a list exactly as a page's own `State:` \
         already can — a plain, unpersisted, in-memory-only store with a `Seq of Text` field is \
         refused whole (\"it uses a store whose state this tier cannot give a record type\") though \
         nothing in `docs/frontend.md` restricts a non-persisted store this way: {:?}",
        result.err()
    );
}

/// G19 (gauntlet2) — a `## Store`'s own action taking TWO OR MORE parameters, whose body reads its
/// own field through a function call, mutates a local copy, and writes the result back (the
/// natural shape of "decode the list, push the new item, re-encode, save" — exactly what a shared
/// cart, a shared chat log, or any shared list-backed store needs), silently never runs when called
/// cross-component (`Call Store's action with a and b.`): no error, no trap, the store's field is
/// left completely unchanged. Confirmed on the reference tree-walker directly, so this is not a
/// WASM-backend-only gap. The SAME body with exactly ONE parameter (dropping `price`, otherwise
/// byte-identical) runs correctly on every tier (`vm_agreed=1` under `largo test`). Bisected from
/// `examples/shopping_cart`'s natural `addLine(name: Text) and (price: Int)`.
///
/// diag2's diagnosis: `Call Cart's addOne with "tea" and 300.` parses its arguments as ONE `Bool`
/// expression, `"tea" and 300` (the sentence's `and` read as the logical operator joining a single
/// argument list, not as the "and" separating two arguments) — so the second parameter is simply
/// never bound. lgxA is fixing the parser; this fixture must go RED for exactly that reason (the
/// second parameter, `price`, unbound) before the fix lands.
#[test]
fn a_two_parameter_store_actions_json_round_trip_actually_writes_back() {
    const TWO_PARAM_STORE_ACTION: &str = r##"
## A CartItem has:
    an id, which is Int.
    a name, which is Text.
    a price, which is Int.

## To fieldOr (j: Json) and (key: Text) -> Json:
    Inspect jsonField(j, key):
        When Ok (v):
            Return v.
        When Err (message):
            Return a new JNull.

## To intFieldOr (j: Json) and (key: Text) and (fallback: Int) -> Int:
    Inspect jsonInt(fieldOr(j, key)):
        When Ok (v):
            Return v.
        When Err (message):
            Return fallback.

## To textFieldOr (j: Json) and (key: Text) and (fallback: Text) -> Text:
    Inspect jsonString(fieldOr(j, key)):
        When Ok (v):
            Return v.
        When Err (message):
            Return fallback.

## To itemToJson (t: CartItem) -> Json:
    Let mutable entries be a new Seq of JEntry.
    Push (a new JEntry with key "id" and val (a new JNum with n t's id)) to entries.
    Push (a new JEntry with key "name" and val (a new JStr with s t's name)) to entries.
    Push (a new JEntry with key "price" and val (a new JNum with n t's price)) to entries.
    Return a new JObj with entries entries.

## To jsonToItem (j: Json) -> CartItem:
    Return a new CartItem with id (intFieldOr(j, "id", 0)) and name (textFieldOr(j, "name", "")) and price (intFieldOr(j, "price", 0)).

## To itemsToJson (items: Seq of CartItem) -> Text:
    Let mutable elems be a new Seq of Json.
    Repeat for t in items:
        Push itemToJson(t) to elems.
    Return jsonSerialize(a new JArr with elems elems).

## To itemsFromJson (itemsJson: Text) -> Seq of CartItem:
    Let mutable out be a new Seq of CartItem.
    Inspect jsonTryParse(itemsJson):
        When JOk (doc):
            Inspect jsonArray(doc):
                When Ok (elems):
                    Repeat for e in elems:
                        Push jsonToItem(e) to out.
                When Err (message):
                    Return out.
        When JErr (message):
            Return out.
    Return out.

## Store Cart

State:
    Let itemsJson be "[]".

Actions:
    To addOne (name: Text) and (price: Int):
        Let mutable current be itemsFromJson(itemsJson).
        Push (a new CartItem with id (length of current + 1) and name name and price price) to current.
        Set itemsJson to itemsToJson(current).

## Page /

Use Cart.

State:
    Let items be a new Seq of CartItem.

Actions:
    When Cart's itemsJson changes:
        Set items to itemsFromJson(Cart's itemsJson).

    To add:
        Call Cart's addOne with "tea" and 300.

View:
    <button id="add" on-click={add}>add</button>
    <p id="len">{length of items}</p>
    <ul>
        {for each item in items:}
            <li>{item's name}: {item's price}</li>
        {end}
    </ul>

## Test: a two-param store action with a full JSON round trip

Click "#add".
Expect that the text of "#len" is equal to "1".
Expect that the text of "li" is equal to "tea: 300".
"##;
    let outcome = one_outcome(TWO_PARAM_STORE_ACTION);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "a `## Store`'s action taking two parameters (`addOne(name: Text) and (price: Int)`), \
         called cross-component with `Call Cart's addOne with \"tea\" and 300.`, must write its \
         JSON-encoded field back exactly as the identical body with ONE parameter already does — \
         instead the second argument, `300`, is never bound: {:?}",
        outcome.failures
    );
}

/// G20 (gauntlet2) — `File`'s fields (`file's name`, `file's size` — the ONLY documented way to
/// read what `When a file is dropped on "#zone" (file: File):` bound, per `docs/frontend.md`'s
/// trigger reference) crash the WASM AOT backend with "unsupported field access with no static
/// slot" — the exact same class G10 already found for `HttpError's` `message` field. This is a
/// STATIC compile-time check (`compile_to_wasm` on a page that never actually fires the trigger);
/// it does not by itself claim the tree-walker or the lowered VM successfully deliver a REAL drop
/// to a `(file: File)` handler and read these fields back — see G25, which found separately that a
/// `(file: File)` handler never runs at all when a drop is actually driven. Same owner and likely
/// the same fix as G10 (`crates/logicaffeine_compile/src/vm/wasm/kind.rs`): a stdlib-materialized
/// struct type (`File`, like `HttpError`) gets a struct NAME but no field-slot LAYOUT.
#[test]
fn a_dropped_files_name_and_size_compile_for_the_browser() {
    use logicaffeine_compile::compile::compile_to_wasm;

    const FILE_FIELD_ACCESS: &str = "\
## Page /

State:
    Let fileName be \"\".
    Let fileSize be 0.

Actions:
    When a file is dropped on \"#zone\" (file: File):
        Set fileName to file's name.
        Set fileSize to file's size.

View:
    <div id=\"zone\">drop here</div>
    <p id=\"name\">{fileName}</p>
    <p id=\"size\">{fileSize}</p>
";
    let result = compile_to_wasm(FILE_FIELD_ACCESS);
    assert!(
        result.is_ok(),
        "`file's name` and `file's size` on a `When a file is dropped on …:` binding are the \
         documented way to read a dropped file and must compile for the browser exactly as they \
         already run correctly on the tree-walker and the lowered bytecode VM: {:?}",
        result.err()
    );
}

/// G22 (gauntlet2) — the mere PRESENCE of a `When a message arrives on "topic" (m: T):` handler
/// delivers ONE SPURIOUS message with an empty payload on the lowered VM tier — the tier a real
/// browser runs — even when `Publish … on "topic".` is never reached at all (guarded out by an
/// `If` that is never true in this fixture). `docs/frontend.md`'s own Triggers section teaches
/// `Publish`/`When a message arrives` as the documented cross-page real-time messaging mechanism
/// ("Two pages of an app — or two tabs — talk over a topic"), and building `examples/chat`'s
/// composer hits this immediately: submitting an empty draft (which correctly does nothing on the
/// tree-walker) makes an empty row appear in the message log on the lowered VM tier. Bisected to
/// the smallest possible shape: an action with an `If` guard that is never satisfied, a page-level
/// `When a message arrives on "…" (m: Text):` handler, and one click — no `Publish` call is ever
/// reached, and the lowered VM tier still runs the handler once with an empty `m`.
#[test]
fn a_message_topic_handler_does_not_fire_on_its_own() {
    const SUBSCRIBING_WITHOUT_PUBLISHING: &str = r##"
## Page /

State:
    Let draft be "".
    Let messages be a new Seq of Text.

Actions:
    To send:
        If draft is not equal to "":
            Publish draft on "chat-room".

    When a message arrives on "chat-room" (m: Text):
        Push m to messages.

View:
    <ul id="log">
        {for each m in messages, at index i:}
            <li id="msg-{i}">{m}</li>
        {else:}
            <li class="empty">No messages yet.</li>
        {end}
    </ul>
    <button id="go" on-click={send}>Send</button>

## Test: clicking with an empty draft sends nothing

Click "#go".
Expect that the text of ".empty" is equal to "No messages yet.".
"##;
    let outcome = one_outcome(SUBSCRIBING_WITHOUT_PUBLISHING);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "declaring `When a message arrives on \"chat-room\" (m: Text):` must not, by itself, \
         deliver a spurious empty message on the lowered VM tier when `Publish` is never reached — \
         the tree-walker correctly shows an empty log; the lowered VM tier (what a real browser \
         runs) shows one empty row instead: {:?}",
        outcome.failures
    );
}

/// G23 (gauntlet2) — `Call <name>.` (no possessive) silently does nothing when `<name>` is one of
/// the CURRENT component's own `Actions:`, on the reference tree-walker itself (not a WASM-only
/// gap). `docs/frontend.md`'s only documented `Call` form is `Call Store's add.` (always through a
/// possessive, always a STORE's action); nothing says a plain action cannot factor a shared step
/// into a helper action the way `examples/data_table` naturally reached for
/// (`To refresh:` called from `setDept`/`sortById`/`nextPage`/…) — `largo check` reports success,
/// and the button visibly does nothing (the visitor sees no error, and NEITHER of two chained
/// `Call bump.` statements below runs `bump` even once).
#[test]
fn calling_a_same_components_own_action_by_its_bare_name_runs_it() {
    const BARE_CALL_TO_OWN_ACTION: &str = r##"
## Page /

State:
    Let n be 0.

Actions:
    To bumpTwice:
        Call bump.
        Call bump.

    To bump:
        Set n to n + 1.

View:
    <button id="go" on-click={bumpTwice}>go</button>
    <p id="n">{n}</p>

## Test: calling a same-page action by its bare name runs it

Click "#go".
Expect that the text of "#n" is equal to "2".
"##;
    let outcome = one_outcome(BARE_CALL_TO_OWN_ACTION);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "`Call bump.` must invoke this component's OWN `To bump:` action exactly as `Call Store's \
         add.` invokes a store's, so two chained calls run it twice — instead `n` never changes at \
         all: {:?}",
        outcome.failures
    );
}

/// G25 (gauntlet2) — `When a file is dropped on "#zone" (file: File):`, the ONLY signature
/// `docs/frontend.md`'s trigger reference documents, never runs its body at all when a drop
/// actually happens, on the reference tree-walker itself. `docs/frontend.md`'s own Testing
/// reference lists no drive verb for a file drop; the only way this campaign found to drive one AT
/// ALL is the UNDOCUMENTED `Raise "filedropped" on "#sel" with "<text>".` (found only by reading
/// `lgx_vm_parity_lock.rs`'s own `a_dropped_file_reaches_its_handler_with_its_name_on_both_tiers`),
/// which delivers to a `(name: Text)` handler — never to a `(file: File)` one, which this test
/// proves never fires at all (its `hits` counter never leaves 0). Building
/// `examples/file_drop_upload`, this makes the documented signature completely unusable: it cannot
/// be tested by any means this lane could find, and separately (G20) cannot compile for the
/// browser. `Upload <file> to url as T, then action.` needs a real file handle `(name: Text)`
/// cannot provide (a bare `Upload name to …` where `name` is the dropped file's Text name compiles
/// but its callback never runs either), so "drop a file, then upload it" has no working path
/// through the documented API today.
/// G27 (gauntlet2) — `docs/frontend.md`'s own canonical Fetching example, copied verbatim (byte
/// for byte, via a plain string literal — not a hand reduction, and independently reproduced with
/// the CLI's `largo check` in a fresh scratch project), fails to compile for the browser with an
/// UNLOCATED "cannot instantiate a written type: a type the checker could not determine"
/// (`span: 0..0`) at the `compile_to_wasm` API level directly, bypassing the CLI/project layer
/// entirely. `examples/fullstack_todo`'s own, more elaborate typed-fetch page compiles cleanly with
/// the same `Result of Seq of <record> and HttpError` shape, so this is not "typed fetch is
/// broken" outright — building `examples/infinite_scroll`, careful ONE-CHANGE-AT-A-TIME bisection
/// (never a bulk rewrite) starting from `examples/fullstack_todo`'s own working file found no
/// single ingredient whose removal alone explains the gap, and the SAME reduced content flipped
/// between compiling and failing at different points while this lane was working, with no change
/// to the `largo` binary in between — consistent with the checker area itself
/// (`crates/logicaffeine_compile/src/analysis/{check,unify}.rs`, generic/`InferType::Instance`
/// resolution) being live, in-progress work. Recorded here rather than root-caused, since it is not this lane's
/// file to fix; `largo test` (the lowered bytecode VM, not the WASM AOT path) runs and agrees with
/// the tree-walker on the identical page throughout, so this is scoped to WASM/AOT instantiation
/// specifically.
#[test]
fn the_canonical_fetching_example_compiles_for_the_browser() {
    use logicaffeine_compile::compile::compile_to_wasm;
    const DOCS_VERBATIM: &str = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Page /

State:
    Let todos be a new Seq of Todo.
    Let status be \"idle\".

Actions:
    To load:
        Set status to \"loading\".
        Get \"/api/todos\" as Seq of Todo, then loaded.

    To loaded (answer: Result of Seq of Todo and HttpError):
        Inspect answer:
            When Ok (list):
                Set todos to list.
                Set status to \"loaded\".
            When Err (e):
                Set status to \"failed\".

View:
    <button id=\"load\" on-click={load}>Load</button>
    <p id=\"status\">{status}</p>
    <ul>{for each t in todos, keyed by t's id:}<li>{t's text}</li>{end}</ul>
";
    let result = compile_to_wasm(DOCS_VERBATIM);
    assert!(
        result.is_ok(),
        "docs/frontend.md's own canonical Fetching example, copied verbatim, must compile for the \
         browser: {:?}",
        result.err()
    );
}

#[test]
fn a_file_typed_drop_handler_runs_when_a_file_is_actually_dropped() {
    const FILE_TYPED_HANDLER: &str = r##"
## Page /

State:
    Let hits be 0.
    Let fileName be "".

Actions:
    When a file is dropped on "#zone" (file: File):
        Set hits to hits + 1.
        Set fileName to file's name.

View:
    <div id="zone">drop here</div>
    <p id="hits">{hits}</p>
    <p id="name">{fileName}</p>

## Test: a real drop reaches a File-typed handler

Raise "filedropped" on "#zone" with "photo.png".
Expect that the text of "#hits" is equal to "1".
Expect that the text of "#name" is equal to "photo.png".
"##;
    let outcome = one_outcome(FILE_TYPED_HANDLER);
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "`When a file is dropped on \"#zone\" (file: File):` — the only signature \
         `docs/frontend.md` documents — must run when a file is actually dropped, exactly as the \
         internally-tested `(name: Text)` signature already does: {:?}",
        outcome.failures
    );
}
