//! ★ LOCK — a `## Store` reaches every tier under ANY name the author chooses. ★
//!
//! A store lowers to one record, `__Store_<Name>`, and the lowering is the one place that record is
//! typed and registered: each `State:` field by the rule that builds it (`a new Seq of Text` keeps
//! its element), under the name the author spelled. The engines lay the store out from the
//! registry as the parse leaves it (`Parser::into_type_registry`), so the tree-walker, the lowered
//! VM and the WebAssembly backend read that one record, whole.
//!
//! An identifier the author chose is never reinterpreted by the dictionary, so this file holds the
//! property over names of every lexical class: nouns the lexicon knows singular and plural, a word
//! it does not know, and words it knows as a verb, an adjective and a keyword. Every name runs
//! identically on the reference tier and the lowered VM (`run_tests`), builds for the browser, and
//! runs on the browser module itself: the store's action, dispatched on the wasm host, journals the
//! same field changes the reference does.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::semantics::world::{self, JournalEntry};
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_language::analysis::registry::{FieldType, TypeDef};
use logicaffeine_language::Interner;

#[path = "compile_common/mod.rs"]
mod common;

/// Every lexical class a store's name can fall into, with the class written beside it so a
/// failure names what the lexicon made of the word.
const NAMES: &[(&str, &str)] = &[
    ("Cart", "a singular noun the lexicon knows"),
    ("Box", "a singular noun the lexicon also knows as a verb"),
    ("Person", "a singular noun with an irregular plural"),
    ("Tasks", "a plural noun the lexicon knows"),
    (
        "Toasts",
        "a plural noun the lexicon knows (the name examples/modal_toast uses)",
    ),
    ("Carts", "a plural of a word the lexicon may not know"),
    ("Notes", "a plural noun the lexicon also knows as a verb"),
    ("Zqx", "a word the lexicon does not know"),
    ("Run", "a verb"),
    ("Open", "a verb and an adjective"),
    ("Red", "an adjective"),
    ("Items", "the indexing keyword"),
    ("Store", "the block's own word"),
];

/// The page part of the fixture: a two-field store (a `Seq` and a scalar) named `name`, one
/// action that moves both fields, and a page that shows both.
fn store_page(name: &str) -> String {
    format!(
        r##"## Store {name}

State:
    Let names be a new Seq of Text.
    Let count be 0.

Actions:
    To bump:
        Push "x" to names.
        Set count to count + 1.

## Page /

Use {name}.

Actions:
    To go:
        Call {name}'s bump.

View:
    <main>
      <button id="go" on-click={{go}}>go</button>
      <p id="count">{{{name}'s count}}</p>
      <p id="len">{{length of {name}'s names}}</p>
      <ul id="names">{{for each t in {name}'s names:}}<li>{{t}}</li>{{end}}</ul>
    </main>
"##
    )
}

/// The page with the `## Test` every tier is held to.
fn tested(name: &str) -> String {
    format!(
        "{}\n## Test: the store named {name} moves on every tier\n\
         Click \"#go\".\n\
         Click \"#go\".\n\
         Expect that the text of \"#count\" is equal to \"2\".\n\
         Expect that the text of \"#len\" is equal to \"2\".\n",
        store_page(name)
    )
}

/// `source` passes its own `## Test` on the reference tier, traces identically on the lowered VM,
/// and builds for the browser.
fn passes_everywhere(name: &str, class: &str, source: &str) -> Vec<u8> {
    let report = run_tests(source, &TestConfig::default()).unwrap_or_else(|e| {
        panic!("`## Store {name}` ({class}) must parse and run: {e}\n{source}")
    });
    assert!(
        !report.outcomes.is_empty(),
        "the fixture declares one ## Test:\n{source}"
    );
    for outcome in &report.outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`## Store {name}` ({class}): `{}` must pass on the reference tier and trace identically \
             on the lowered VM: {:?}",
            outcome.name,
            outcome.failures
        );
    }
    compile_to_wasm(source).unwrap_or_else(|e| {
        panic!("`## Store {name}` ({class}) must build for the browser exactly as it runs on the other tiers: {e:?}\n{source}")
    })
}

/// The reference tier's journal for `source` driven by `steps` under `## Main`, as
/// `(action, changes, failure)` — the shape the browser host's `noteTurn` carries.
fn reference_journal(source: &str, steps: &[&str]) -> Vec<(String, Vec<String>, String)> {
    dom::reset();
    world::reset();
    let program = format!("{source}\n## Main\n{}\n", steps.join("\n"));
    let out = tw_outcome(&program);
    assert_eq!(
        out.error, None,
        "the reference tier runs the driven page:\n{program}"
    );
    world::journal().iter().map(flat).collect()
}

fn flat(e: &JournalEntry) -> (String, Vec<String>, String) {
    let changes = e
        .changes
        .iter()
        .flat_map(|(f, b, a)| [f.clone(), b.clone(), a.clone()])
        .collect();
    (
        e.action.clone(),
        changes,
        e.failed.clone().unwrap_or_default(),
    )
}

/// The browser module, mounted on the wasm host and clicked twice: its journal, in the same shape.
fn browser_journal(name: &str, class: &str, source: &str) -> Vec<(String, Vec<String>, String)> {
    let mut page = common::wasm_host::mount_interactive(source);
    let button = *page.listening("click").last().unwrap_or_else(|| {
        panic!("`## Store {name}` ({class}): the page registered its click listener")
    });
    for _ in 0..2 {
        let heard = page.dispatch(button, "click", "").unwrap_or_else(|e| {
            panic!("`## Store {name}` ({class}): the click's turn runs on the browser module: {e}")
        });
        assert!(
            heard,
            "`## Store {name}` ({class}): the click reached the page's listener"
        );
    }
    page.journal()
        .into_iter()
        .map(|c| (c.name, c.changes, c.failure))
        .collect()
}

/// A field's type as the registry spells it: `Seq of Text`, `Int`.
fn spelled(ty: &FieldType, interner: &Interner) -> String {
    match ty {
        FieldType::Primitive(s) | FieldType::Named(s) | FieldType::TypeParam(s) => {
            interner.resolve(*s).to_string()
        }
        FieldType::Generic { base, params } => {
            let params: Vec<String> = params.iter().map(|p| spelled(p, interner)).collect();
            format!("{} of {}", interner.resolve(*base), params.join(" and "))
        }
        other => format!("{other:?}"),
    }
}

/// The registry an engine lays the store out from holds the store's record WHOLE, under the name
/// the author spelled, with the `Seq` field's element kept — the lowering's own record, and no
/// other record for the store under any name a dictionary made of the word. This is the half of
/// the property the runs above cannot see on their own: with the record absent an engine recovers
/// a layout from Main's construction, and with a partial record it does not.
#[test]
fn the_engines_receive_the_stores_record_whole_under_every_name() {
    for (name, class) in NAMES {
        let page = store_page(name);
        with_parsed_program(&page, |parsed, interner| {
            let (_, registry, _) = parsed
                .unwrap_or_else(|e| panic!("`## Store {name}` ({class}) must parse: {e}\n{page}"));
            let record_name = format!("__Store_{name}");
            let record = interner
                .lookup(&record_name)
                .and_then(|sym| registry.get(sym));
            let Some(TypeDef::Struct { fields, .. }) = record else {
                panic!("`## Store {name}` ({class}): the engines' registry must hold `{record_name}` as a record, not {record:?}");
            };
            let shape: Vec<(String, String)> = fields
                .iter()
                .map(|f| {
                    (
                        interner.resolve(f.name).to_string(),
                        spelled(&f.ty, interner),
                    )
                })
                .collect();
            let want = [
                ("names".to_string(), "Seq of Text".to_string()),
                ("count".to_string(), "Int".to_string()),
            ];
            assert_eq!(shape, want, "`## Store {name}` ({class}): `{record_name}` carries every field, each with its whole type");
            let store_records: Vec<String> = registry
                .iter_types()
                .map(|(sym, _)| interner.resolve(*sym).to_string())
                .filter(|record| record.starts_with("__Store_"))
                .collect();
            assert_eq!(
                store_records,
                [record_name.clone()],
                "`## Store {name}` ({class}): the registry holds the store's record once, under the name the author \
                 spelled, and none under a name the lexicon made of the word"
            );
        });
    }
}

#[test]
fn a_two_field_store_runs_identically_on_every_tier_under_every_name() {
    for (name, class) in NAMES {
        let source = tested(name);
        passes_everywhere(name, class, &source);
    }
}

#[test]
fn the_browser_module_journals_the_stores_action_as_the_reference_does_under_every_name() {
    for (name, class) in NAMES {
        let page = store_page(name);
        let reference = reference_journal(&page, &["Click \"#go\".", "Click \"#go\"."]);
        let bumps: Vec<_> = reference
            .iter()
            .filter(|(action, _, _)| action == "bump")
            .collect();
        assert_eq!(
            bumps.len(),
            2,
            "`## Store {name}` ({class}): the reference journals both bumps: {reference:?}"
        );
        for (k, (_, changes, failure)) in bumps.iter().enumerate() {
            let fields: Vec<&str> = changes.chunks(3).map(|c| c[0].as_str()).collect();
            assert_eq!(
                fields,
                ["names", "count"],
                "`## Store {name}` ({class}): bump {} moves the Seq field and the scalar field: {changes:?}",
                k + 1
            );
            assert_eq!(
                failure,
                "",
                "`## Store {name}` ({class}): bump {} commits",
                k + 1
            );
        }
        let browser = browser_journal(name, class, &page);
        assert_eq!(
            browser, reference,
            "`## Store {name}` ({class}): the browser module journals every turn — the store's action \
             with both of its fields — exactly as the reference tier does"
        );
    }
}

/// A `persisted as` store is the same record read back from browser storage; its name is no
/// more the lexicon's business than a local store's.
#[test]
fn a_persisted_two_field_store_runs_identically_on_every_tier_under_every_name() {
    for (name, class) in NAMES {
        let source = tested(name).replacen(
            &format!("## Store {name}\n"),
            &format!(
                "## Store {name} persisted as \"{}\"\n",
                name.to_ascii_lowercase()
            ),
            1,
        );
        assert!(
            source.contains("persisted as"),
            "the fixture persists its store:\n{source}"
        );
        passes_everywhere(name, class, &source);
    }
}
