//! ★ LOCK — browser storage is REAL storage, and a store that says it persists, persists. ★
//!
//! `## Store Session persisted as "session"` compiled to WebAssembly and kept NOTHING. There was
//! no storage host import anywhere in the ABI, so on the one tier that runs in a browser the
//! store's state lived in the module's own memory and died with the page. The sentence said
//! "persisted"; a reload started from the initializers. That is the failure this campaign keeps
//! finding — a declaration the runtime does not honour — and it is at its worst here, because
//! what a persisted store holds is precisely the thing the visitor typed.
//!
//! THE SURFACE IS THREE OPERATIONS AND AN AREA. `storageSet(area, key, value)`,
//! `storageGet(area, key)`, `storageRemove(area, key)`, where the area is `"local"` (survives
//! closing the tab — what "persisted" means) or `"session"` (survives a RELOAD and dies with the
//! tab — what "preserved across reloads" means). The area is an ARGUMENT rather than two families
//! of import so that adding a third one later is not a change to the boundary; an area nobody
//! defines is refused by name rather than silently written somewhere.
//!
//! A MISSING KEY IS `none`, NEVER `""`. An empty string is a value a page can legitimately store
//! — an empty draft is exactly what a form field holds — and a reader that cannot tell "nothing
//! is saved" from "an empty string is saved" will overwrite one with the other.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers, and they must agree — a page's storage is written by the browser tier and this is
/// where the reference tier's meaning is pinned to it.
fn both(src: &str) -> String {
    logicaffeine_compile::semantics::world::reset();
    logicaffeine_compile::semantics::dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(
        tw.error, None,
        "the program must run on the tree-walker:\n{src}"
    );
    logicaffeine_compile::semantics::world::reset();
    logicaffeine_compile::semantics::dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the program must run on the VM:\n{src}");
    // Trailing newline only: the two harnesses differ by one, everywhere, and always have.
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "tree-walker and VM disagreed about storage:\n{src}"
    );
    tw.output
}

fn tw_error(src: &str) -> String {
    logicaffeine_compile::semantics::world::reset();
    let out = tw_outcome(src);
    out.error
        .unwrap_or_else(|| panic!("expected a refusal, got output {:?}:\n{src}", out.output))
}

#[test]
fn a_value_saved_comes_back() {
    let out = both(
        "## Main\n\
         Call storageSet with \"local\" and \"draft\" and \"half-written\".\n\
         Show storageGet(\"local\", \"draft\").\n",
    );
    assert_eq!(out.trim(), "half-written");
}

#[test]
fn a_missing_key_is_none_and_not_an_empty_string() {
    // The edge case the whole read path turns on. A page that stores an empty draft and a page
    // that has stored nothing are DIFFERENT, and a reader that conflates them either resurrects
    // a value nobody saved or throws away one somebody did.
    let out = both(
        "## Main\n\
         Call storageSet with \"local\" and \"present\" and \"\".\n\
         Inspect storageGet(\"local\", \"absent\"):\n\
        \x20   When Some (v):\n\
        \x20       Show \"absent: some\".\n\
        \x20   Otherwise:\n\
        \x20       Show \"absent: none\".\n\
         Inspect storageGet(\"local\", \"present\"):\n\
        \x20   When Some (v):\n\
        \x20       Show \"present: some\".\n\
        \x20   Otherwise:\n\
        \x20       Show \"present: none\".\n",
    );
    assert_eq!(out.trim(), "absent: none\npresent: some");
}

#[test]
fn forgetting_a_key_removes_it() {
    let out = both(
        "## Main\n\
         Call storageSet with \"local\" and \"k\" and \"v\".\n\
         Call storageRemove with \"local\" and \"k\".\n\
         Inspect storageGet(\"local\", \"k\"):\n\
        \x20   When Some (v):\n\
        \x20       Show \"still there\".\n\
        \x20   Otherwise:\n\
        \x20       Show \"gone\".\n",
    );
    assert_eq!(out.trim(), "gone");
}

#[test]
fn removing_a_key_that_was_never_there_is_not_an_error() {
    // A page clearing state it may or may not have written must not have to ask first.
    let out = both(
        "## Main\n\
         Call storageRemove with \"session\" and \"never\".\n\
         Show \"ok\".\n",
    );
    assert_eq!(out.trim(), "ok");
}

#[test]
fn the_two_areas_do_not_see_each_other() {
    // `local` and `session` are different browser objects, and the difference is the whole point:
    // one survives closing the tab and one does not. A key written to one must not be readable
    // from the other, or "preserved across reloads" would quietly outlive the session it named.
    let out = both(
        "## Main\n\
         Call storageSet with \"local\" and \"k\" and \"long-lived\".\n\
         Call storageSet with \"session\" and \"k\" and \"this-tab\".\n\
         Show storageGet(\"local\", \"k\").\n\
         Show storageGet(\"session\", \"k\").\n",
    );
    assert_eq!(out.trim(), "long-lived\nthis-tab");
}

#[test]
fn a_later_write_replaces_the_earlier_one() {
    let out = both(
        "## Main\n\
         Call storageSet with \"session\" and \"k\" and \"first\".\n\
         Call storageSet with \"session\" and \"k\" and \"second\".\n\
         Show storageGet(\"session\", \"k\").\n",
    );
    assert_eq!(out.trim(), "second");
}

#[test]
fn reading_with_a_fallback_answers_the_stored_value_when_there_is_one() {
    let out = both(
        "## Main\n\
         Call storageSet with \"local\" and \"k\" and \"saved\".\n\
         Show storageGetOr(\"local\", \"k\", \"default\").\n\
         Show storageGetOr(\"local\", \"absent\", \"default\").\n",
    );
    assert_eq!(out.trim(), "saved\ndefault");
}

#[test]
fn a_stored_empty_string_beats_the_fallback() {
    // The edge case that decides whether `storageGetOr` is honest. An empty string is a VALUE —
    // a cleared form field stores exactly that — so a page that saved one and reads it back must
    // get it, not the default it started from. A `getItem(k) || fallback` would get this wrong,
    // silently, and only for the value a visitor produces by clearing a field.
    let out = both(
        "## Main\n\
         Call storageSet with \"local\" and \"k\" and \"\".\n\
         Let v be storageGetOr(\"local\", \"k\", \"default\").\n\
         Show \"[{v}]\".\n",
    );
    assert_eq!(out.trim(), "[]");
}

#[test]
fn the_fallback_is_answered_for_an_unwritten_area_key_pair() {
    // The area is part of a key's identity here too: the same key in the OTHER area is not this
    // key, so the fallback stands.
    let out = both(
        "## Main\n\
         Call storageSet with \"local\" and \"k\" and \"local-only\".\n\
         Show storageGetOr(\"session\", \"k\", \"nothing in this tab\").\n",
    );
    assert_eq!(out.trim(), "nothing in this tab");
}

#[test]
fn an_area_nobody_defines_is_refused_by_name() {
    // Not a silent no-op and not a third area invented on the spot: a page written against an
    // area that does not exist would store its state where nothing ever reads it.
    let err = tw_error("## Main\nCall storageSet with \"disk\" and \"k\" and \"v\".\n");
    assert!(
        err.contains("disk"),
        "the refusal must name the area it was given: {err}"
    );
    assert!(
        err.contains("local") && err.contains("session"),
        "the refusal must name the areas that do exist: {err}"
    );
}

#[test]
fn hostile_text_round_trips_unchanged() {
    // Storage carries whatever the page put in it. Quotes, newlines, backslashes and multi-byte
    // text are ordinary content, and a codec that mangles any of them corrupts a draft silently.
    //
    // ★ READ THROUGH `storageGetOr`, NOT `storageGet`. This used to compare `storageGet(…)`
    // directly against the Text, which worked only while an `Option` WAS its payload. An Option
    // now carries a real tag, so `some x` and `x` are different values and that comparison is
    // false however faithfully storage round-tripped — which said MANGLED about a byte-perfect
    // round trip. The subject here is the CODEC, not the Option, so this reads the value out the
    // way a page would (`storageGetOr` is the sibling that exists for exactly this) and the
    // hostile bytes are still compared byte for byte.
    let out = both(
        "## Main\n\
         Let hostile be text_from_bytes([34, 92, 10, 9, 226, 152, 131, 60, 47, 62]).\n\
         Call storageSet with \"local\" and \"k\" and hostile.\n\
         If storageGetOr(\"local\", \"k\", \"\") is hostile:\n\
        \x20   Show \"identical\".\n\
         If storageGetOr(\"local\", \"k\", \"\") is not hostile:\n\
        \x20   Show \"MANGLED\".\n",
    );
    assert_eq!(out.trim(), "identical");
}

#[test]
fn an_empty_key_is_a_key() {
    // Nothing about the empty string makes it a non-key, and a store keyed by a computed name
    // may legitimately produce one. Refusing it here would fail far from its cause.
    let out = both(
        "## Main\n\
         Call storageSet with \"local\" and \"\" and \"v\".\n\
         Show storageGet(\"local\", \"\").\n",
    );
    assert_eq!(out.trim(), "v");
}

// ---------------------------------------------------------------------------------------------
// A store that says it persists, persists — through the SAME storage, on both tiers.
// ---------------------------------------------------------------------------------------------

const PERSISTED: &str = "\
## Store Draft persisted as \"draft-key\"
State:
    Let text be \"\".
Actions:
    To type (t: Text):
        Set text to t.

## Component App
Use Draft.
Actions:
    To fill:
        Call Draft's type with \"typed\".
View:
    <p id=\"t\">{Draft's text}</p>
    <button id=\"go\" on-click={fill}>x</button>

## Main
Mount App.
";

#[test]
fn a_persisted_store_writes_through_to_storage() {
    // The load-bearing one. A persisted store's state must be READABLE FROM STORAGE — that is
    // what makes it survive a page load, and it is exactly what did not happen before.
    let out = both(&format!(
        "{PERSISTED}Click \"#go\".\n\
         Inspect storageGet(\"local\", \"draft-key\"):\n\
        \x20   When Some (v):\n\
        \x20       Show \"stored\".\n\
        \x20   Otherwise:\n\
        \x20       Show \"NOTHING WAS STORED\".\n"
    ));
    assert_eq!(out.trim(), "stored");
}

#[test]
fn a_preserved_store_uses_session_storage() {
    // `preserved across reloads` means exactly the lifetime `sessionStorage` has: it survives a
    // reload and dies with the tab. Writing it to `localStorage` would leave a developer's stale
    // draft outliving the session that made it, on a visitor's machine.
    let src = "\
## Store Draft preserved across reloads
State:
    Let text be \"\".
Actions:
    To type (t: Text):
        Set text to t.

## Component App
Use Draft.
Actions:
    To fill:
        Call Draft's type with \"typed\".
View:
    <p id=\"t\">{Draft's text}</p>
    <button id=\"go\" on-click={fill}>x</button>

## Main
Mount App.
Click \"#go\".
Inspect storageGet(\"session\", \"reload:Draft\"):
    When Some (v):
        Show \"in the session\".
    Otherwise:
        Show \"NOTHING WAS STORED\".
Inspect storageGet(\"local\", \"reload:Draft\"):
    When Some (v):
        Show \"LEAKED TO LOCAL\".
    Otherwise:
        Show \"not in local\".
";
    logicaffeine_compile::semantics::world::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the page must run:\n{src}");
    assert_eq!(out.output.trim(), "in the session\nnot in local");
}

#[test]
fn a_local_store_writes_nothing_to_storage() {
    // Storage is what a store OPTS INTO. A local store leaving keys behind would put a page's
    // working state on a visitor's disk without the page ever saying so.
    let out = both(
        "## Store Draft\n\
         State:\n\
        \x20   Let text be \"\".\n\
         Actions:\n\
        \x20   To type (t: Text):\n\
        \x20       Set text to t.\n\
         \n\
         ## Component App\n\
         Use Draft.\n\
         Actions:\n\
        \x20   To fill:\n\
        \x20       Call Draft's type with \"typed\".\n\
         View:\n\
        \x20   <p id=\"t\">{Draft's text}</p>\n\
        \x20   <button id=\"go\" on-click={fill}>x</button>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#go\".\n\
         Inspect storageGet(\"local\", \"Draft\"):\n\
        \x20   When Some (v):\n\
        \x20       Show \"LEAKED\".\n\
        \x20   Otherwise:\n\
        \x20       Show \"nothing stored\".\n",
    );
    assert_eq!(out.trim(), "nothing stored");
}

#[test]
fn a_persisted_store_holding_something_text_cannot_carry_says_so() {
    // A store that keeps its state between page loads carries every field THROUGH TEXT: a
    // scalar's own tag, or the derived JSON codec for a record it can encode AND decode. A
    // `Ledger` holds a `Map`, which the codec has no form for, so the store cannot be persisted —
    // and the refusal must say THAT.
    //
    // ★ IT MUST NOT BORROW ANOTHER REASON. "a store whose state this tier cannot give a record
    // type" is a different problem with a different fix and sends the reader to look at their
    // type annotations. Naming the wrong cause is worse than naming none: it costs the reader the
    // time it takes to rule out the thing you told them. The field's type is a named record — so
    // the record-shape half is satisfied and the only thing left is that a `Ledger` has no text
    // form. That separation is the point of the fixture: a bare `Map` field would fail BOTH checks
    // and could not tell the messages apart.
    let src = "\
## A Ledger has:
    a counts, which is Map of Text to Int.

## Store Account persisted as \"account\"
State:
    Let ledger: Ledger be a new Ledger.
Actions:
    To reset:
        Set ledger to a new Ledger.

## Component App
Use Account.
View:
    <p>{length of Account's ledger's counts}</p>

## Main
Mount App.
";
    let err = format!(
        "{:?}",
        logicaffeine_compile::compile::compile_to_program(src).err()
    );
    assert!(
        err.contains("keeps its state between page loads")
            && err.contains("something text cannot carry"),
        "the refusal must name the real cause — a field with no text form: {err}"
    );
    assert!(
        !err.contains("cannot give a record type"),
        "and must not borrow the record-typing reason: {err}"
    );
}

/// A persisted store's RECORD field goes through the derived JSON codec, so it survives a reload
/// like any scalar — on the reference, on the lowered page in-process, and in the browser build.
const PERSISTED_RECORD: &str = r##"## A Profile has:
    a handle, which is Text.

## Store Account persisted as "account"
State:
    Let owner: Profile be a new Profile with handle "ada".
Actions:
    To rename (h: Text):
        Set owner to a new Profile with handle h.

## Page /
Use Account.
Actions:
    To go:
        Call Account's rename with "grace".
View:
    <p id="h">{Account's owner's handle}</p>
    <button id="go" on-click={go}>go</button>

## Test: a persisted record survives a reload
    Expect that the text of "#h" is equal to "ada".
    Click "#go".
    Reload the page.
    Expect that the text of "#h" is equal to "grace".
"##;

#[test]
fn a_persisted_store_holding_a_record_survives_a_reload_on_every_tier() {
    use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
    let report = run_tests(PERSISTED_RECORD, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the page must parse and run: {e}"));
    assert_eq!(report.outcomes.len(), 1, "one scenario");
    for outcome in &report.outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`{}`: {:?}",
            outcome.name,
            outcome.failures
        );
    }
    let built = logicaffeine_compile::compile::compile_to_wasm(PERSISTED_RECORD);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

#[test]
fn a_local_store_holding_a_seq_field_works_on_every_tier() {
    // The other half of the split. A store this tier cannot TYPE and a store it cannot CARRY
    // THROUGH TEXT were different problems with different fixes; the typing half is gone — a store
    // field is typed like any page field, a `Seq` included — so the same page as a LOCAL store is
    // simply a page. It renders the same list on every tier, and, being local, it still stores
    // nothing: persistence was never its trouble.
    let shape = "\
## Store Cart
State:
    Let items be a new Seq of Int.
Actions:
    To add:
        Push 1 to items.

## Component App
Use Cart.
Actions:
    To buy:
        Call Cart's add.
View:
    <p id=\"n\">{length of Cart's items}</p>
    <button id=\"b\" on-click={buy}>buy</button>

## Main
Mount App.
Click \"#b\".
Click \"#b\".
";
    let want = "<body><p id=\"n\">2</p><button id=\"b\">buy</button></body>";
    for (tier, run) in [
        (
            "tree-walker",
            tw_outcome as fn(&str) -> logicaffeine_compile::compile::RunOutcome,
        ),
        ("VM", vm_outcome),
    ] {
        logicaffeine_compile::semantics::world::reset();
        logicaffeine_compile::semantics::dom::reset();
        logicaffeine_compile::semantics::storage::reset();
        let out = run(shape);
        assert_eq!(out.error, None, "the page must run on the {tier}:\n{shape}");
        assert_eq!(
            logicaffeine_compile::semantics::dom::render(),
            want,
            "both pushes reach the store's list on the {tier}"
        );
        for area in [
            logicaffeine_compile::semantics::storage::LOCAL,
            logicaffeine_compile::semantics::storage::SESSION,
        ] {
            assert_eq!(
                logicaffeine_compile::semantics::storage::keys(area),
                Ok(Vec::new()),
                "a LOCAL store keeps nothing in {area} storage on the {tier}"
            );
        }
    }
    assert!(
        logicaffeine_compile::compile::compile_to_wasm(shape).is_ok(),
        "and the page builds for the browser: {:?}",
        logicaffeine_compile::compile::compile_to_wasm(shape).err()
    );
}

#[test]
fn the_same_field_is_fine_in_a_local_store() {
    // The persistence refusal must be about PERSISTENCE. A record-typed field a persisted store
    // cannot carry is perfectly ordinary in a local one, and a page using it runs on both tiers —
    // so the refusal is narrow rather than a blanket ban on the field.
    let src = "## A Profile has:\n\
        \x20   a handle, which is Text.\n\
         \n\
         ## Store Account\n\
         State:\n\
        \x20   Let owner: Profile be a new Profile with handle \"ada\".\n\
         Actions:\n\
        \x20   To rename (h: Text):\n\
        \x20       Set owner to a new Profile with handle h.\n\
         \n\
         ## Component App\n\
         Use Account.\n\
         View:\n\
        \x20   <p id=\"h\">{Account's owner's handle}</p>\n\
         \n\
         ## Main\n\
         Mount App.\n";
    // RENDERS on the reference tier…
    logicaffeine_compile::semantics::world::reset();
    logicaffeine_compile::semantics::dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the page runs:\n{src}");
    assert!(
        logicaffeine_compile::semantics::dom::render().contains("ada"),
        "the field renders: {}",
        logicaffeine_compile::semantics::dom::render()
    );
    // …and REACHES the bytecode tier, which is the claim: the persistence refusal is about
    // persistence, not a blanket ban on the field.
    assert!(
        logicaffeine_compile::compile::compile_to_program(src).is_ok(),
        "a LOCAL store with the same field is not refused: {:?}",
        logicaffeine_compile::compile::compile_to_program(src).err()
    );
}

#[test]
fn a_trapping_action_stores_nothing() {
    // ★ ROLLBACK BY CONSTRUCTION COVERS STORAGE TOO, and it has to: a turn that trapped did not
    // happen, so what is on disk must be what the turn STARTED from. Storage is the one place a
    // half-applied turn would OUTLIVE the page that made it — the visitor reloads and the state
    // nobody committed is what comes back, permanently.
    //
    // The write-through is emitted AFTER the action's write-back, so a trap partway through never
    // reaches either. That is the same guarantee the state itself gets, from the same ordering,
    // rather than a second mechanism that could disagree with it.
    let src = "\
## Store Draft persisted as \"draft\"
State:
    Let text be \"start\".
    Let n be 0.
Actions:
    To breakIt (t: Text):
        Set text to t.
        Set n to item 9 of [1].

## Component App
Use Draft.
State:
    Let caught be \"\".
Actions:
    To go:
        Call Draft's breakIt with \"half\".
    When an action fails (e: Text):
        Set caught to \"caught\".
View:
    <p id=\"t\">{Draft's text}</p>
    <p id=\"e\">{caught}</p>
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
Click \"#go\".
Show storageGetOr(\"local\", \"draft.text\", \"NOTHING STORED\").
";
    logicaffeine_compile::semantics::world::reset();
    logicaffeine_compile::semantics::dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the page survives the trap:\n{src}");
    let rendered = logicaffeine_compile::semantics::dom::render();
    assert!(
        rendered.contains(">start<"),
        "the STATE rolled back to what the turn started from: {rendered}"
    );
    assert!(
        !out.output.contains("half"),
        "and STORAGE must not hold the value the trapped turn wrote — a half-applied turn on \
         disk outlives the page that made it: {:?}",
        out.output
    );
}

// ---------------------------------------------------------------------------------------------
// The browser tier: a page that persists must ASK THE HOST to store it.
// ---------------------------------------------------------------------------------------------

#[cfg(feature = "wasm-jit")]
mod browser {
    use super::PERSISTED;
    use logicaffeine_compile::compile::compile_to_wasm;

    fn imports(src: &str) -> Vec<String> {
        let module = compile_to_wasm(src).expect("the page must compile to wasm");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        m.imports().map(|i| i.name().to_string()).collect()
    }

    #[test]
    fn a_page_that_stores_imports_the_hosts_storage() {
        // The gap in one assertion: WebAssembly cannot reach `localStorage` itself, so a module
        // that never asks the host to store anything cannot survive a reload however carefully
        // it manages its own memory. Before this there was no storage import in the whole ABI.
        let names = imports(
            "## Main\n\
             Call storageSet with \"local\" and \"k\" and \"v\".\n\
             Let v be storageGet(\"local\", \"k\").\n\
             If v is not none:\n\
            \x20   Show v.\n\
             Call storageRemove with \"local\" and \"k\".\n",
        );
        for want in ["storage_set", "storage_get", "storage_remove"] {
            assert!(
                names.iter().any(|n| n == want),
                "{want} must be imported: {names:?}"
            );
        }
    }

    #[test]
    fn a_page_with_no_storage_imports_none() {
        // The other half: the import is paid for by pages that use it. A module importing a
        // function its host does not define fails to INSTANTIATE, so a page must not carry an
        // import it never calls.
        let names = imports("## Component App\nView:\n    <p>hi</p>\n\n## Main\nMount App.\n");
        assert!(
            !names.iter().any(|n| n.starts_with("storage_")),
            "a page that stores nothing must not import storage: {names:?}"
        );
    }

    #[test]
    fn a_persisted_store_reaches_the_browser_tier() {
        // ★★ THE WHOLE POINT, on the tier where it matters. A `## Store … persisted as "key"`
        // now compiles to a module that ASKS THE HOST to store its fields and reads them back —
        // which is what makes the sentence true in a browser rather than only on the tiers a
        // visitor never runs.
        //
        // Getting here needed a fix in the general LANGUAGE, not the frontend: this backend
        // could not resolve a field access on a record GLOBAL read inside a FUNCTION, and a
        // store IS a record global read by every action. `wasm_record_global_lock` holds that
        // fix in plain LOGOS, where the gap was actually reachable.
        let names = imports(PERSISTED);
        assert!(
            names.iter().any(|n| n == "storage_set"),
            "a persisted store must write through the host: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "storage_get"),
            "a persisted store must read back through the host: {names:?}"
        );
    }

    #[test]
    fn a_local_store_carries_no_storage_import() {
        // A store keeps nothing between page loads unless it says so. A local one that dragged
        // the storage imports along would be paying for a promise it does not make — and on this
        // tier an unused import is a real cost, since a module importing what its host does not
        // define fails to instantiate.
        let local = PERSISTED.replace(
            "## Store Draft persisted as \"draft-key\"",
            "## Store Draft",
        );
        let names = imports(&local);
        assert!(
            !names.iter().any(|n| n.starts_with("storage_")),
            "a local store must not import storage: {names:?}"
        );
    }
}
