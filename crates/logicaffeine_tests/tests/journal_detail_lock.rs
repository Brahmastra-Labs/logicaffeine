//! LOCK — a lowered turn captures its journal's detail only for a host that keeps it.
//!
//! A lowered turn journals itself through `noteTurn(name, key, changes, failure)`. The changes —
//! `[field, before, after, …]`, each side a field's display — are built only under
//! `journalDetail()`. A host that records the journal (every in-process host, the browser test
//! harness) answers true and receives the reference's triples; a shipped page answers false, and its
//! turns build no display text at all, however much state they hold. The name, key and failure go
//! through either way.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::semantics::world::{self, JournalEntry};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_language::ast::stmt::{Expr, Stmt};
use logicaffeine_language::Interner;

/// Every capture site the lowering has: a component action's turn (with a change handler's watch
/// phase), an own action called from it and inlined, and a store action. The view shows no state,
/// so no paint in any turn formats a field — every display a turn builds is the journal's.
const EVERY_SITE: &str = "\
## Store Tally
State:
    Let hits be 0.
Actions:
    To hit:
        Set hits to hits + 1.

## Component App
Use Tally.
State:
    Let n be 0.
    Let seen be 0.
    Let word be \"zero\".
    Let tags be a new Seq of Text.
Actions:
    To bump:
        Set n to n + 1.
    To go:
        Call bump.
        Set word to \"one\".
        Push \"new\" to tags.
        Call Tally's hit.
    When n changes:
        Set seen to seen + 1.
View:
    <button id=\"go\" on-click={go}>+</button>

## Main
Mount App.
Click \"#go\".
";

/// The displays a block builds — a `Let`, `Set` or `Push` whose value is an interpolation — split by
/// whether an enclosing `If journalDetail():` guards them.
#[derive(Default)]
struct Displays {
    guarded: usize,
    unguarded: Vec<String>,
}

fn is_detail_ask(cond: &Expr, interner: &Interner) -> bool {
    matches!(cond, Expr::Call { function, args } if args.is_empty() && interner.resolve(*function) == "journalDetail")
}

fn displays_in(block: &[Stmt], guarded: bool, interner: &Interner, out: &mut Displays) {
    for s in block {
        let built = match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } | Stmt::Push { value, .. } => {
                matches!(value, Expr::InterpolatedString(_))
            }
            _ => false,
        };
        if built {
            if guarded {
                out.guarded += 1;
            } else {
                out.unguarded.push(format!("{s:?}"));
            }
        }
        match s {
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                displays_in(
                    then_block,
                    guarded || is_detail_ask(cond, interner),
                    interner,
                    out,
                );
                if let Some(e) = else_block {
                    displays_in(e, guarded, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                displays_in(body, guarded, interner, out)
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    displays_in(arm.body, guarded, interner, out);
                }
            }
            _ => {}
        }
    }
}

fn notes_a_turn(block: &[Stmt], interner: &Interner) -> bool {
    block.iter().any(|s| match s {
        Stmt::Call { function, .. } => interner.resolve(*function) == "noteTurn",
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            notes_a_turn(then_block, interner)
                || else_block.is_some_and(|e| notes_a_turn(e, interner))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => notes_a_turn(body, interner),
        Stmt::Inspect { arms, .. } => arms.iter().any(|a| notes_a_turn(a.body, interner)),
        _ => false,
    })
}

#[test]
fn every_display_a_turn_builds_is_built_under_journal_detail() {
    let turns: Vec<(String, Displays)> = with_parsed_program(EVERY_SITE, |parsed, interner| {
        let (stmts, _, _) = parsed.unwrap_or_else(|e| panic!("parses: {e}"));
        stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, body, .. } if notes_a_turn(body, interner) => {
                    let mut displays = Displays::default();
                    displays_in(body, false, interner, &mut displays);
                    Some((interner.resolve(*name).to_string(), displays))
                }
                _ => None,
            })
            .collect()
    });
    for (function, displays) in &turns {
        assert!(
            displays.unguarded.is_empty(),
            "`{function}` builds a display outside `If journalDetail():` — a shipped page pays for it on every turn:\n{}",
            displays.unguarded.join("\n")
        );
    }
    for turn in ["__act_App_go", "__act_App_bump", "__act_Tally_hit"] {
        let displays = turns
            .iter()
            .find(|(f, _)| f == turn)
            .map(|(_, d)| d)
            .unwrap_or_else(|| panic!("`{turn}` journals a turn"));
        assert!(
            displays.guarded > 0,
            "`{turn}` still captures its fields' displays under the ask"
        );
    }
}

fn journal_of(
    run: fn(&str) -> logicaffeine_compile::compile::RunOutcome,
    src: &str,
) -> Vec<JournalEntry> {
    dom::reset();
    world::reset();
    let out = run(src);
    assert_eq!(out.error, None, "the program runs:\n{src}");
    world::journal()
}

#[test]
fn a_host_that_keeps_the_journal_receives_the_references_triples() {
    let reference = journal_of(tw_outcome, EVERY_SITE);
    let lowered = journal_of(vm_outcome, EVERY_SITE);
    let shape = |j: &[JournalEntry]| -> Vec<(
        String,
        String,
        Vec<(String, String, String)>,
        Option<String>,
    )> {
        j.iter()
            .map(|e| {
                (
                    e.instance.clone(),
                    e.action.clone(),
                    e.changes.clone(),
                    e.failed.clone(),
                )
            })
            .collect()
    };
    assert_eq!(
        shape(&lowered),
        shape(&reference),
        "the lowered VM journals every turn exactly as the reference"
    );
    let go = reference
        .iter()
        .find(|e| e.action == "go")
        .expect("the click journals `go`");
    let t = |f: &str, b: &str, a: &str| (f.to_string(), b.to_string(), a.to_string());
    assert_eq!(
        go.changes,
        [
            t("n", "0", "1"),
            t("seen", "0", "1"),
            t("word", "zero", "one"),
            t("tags", "[]", "[new]")
        ],
        "the action's writes and its change handler's, each with both sides — a list's too"
    );
}

#[cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

/// The browser test harness answers `journalDetail()` true (`InteractivePage` records the journal
/// whole), so the compiled page hands it every triple.
#[cfg(feature = "wasm-jit")]
#[test]
fn the_browser_harness_receives_the_triples() {
    let page_src = EVERY_SITE
        .split("## Main")
        .next()
        .expect("the fixture has a page part")
        .to_string()
        + "## Main\nMount App.\n";
    let mut page = common::wasm_host::mount_interactive(&page_src);
    let button = *page
        .listening("click")
        .last()
        .expect("the page registered its click listener");
    page.dispatch(button, "click", "")
        .expect("the click's turn runs");
    let go = page
        .journal()
        .into_iter()
        .find(|c| c.name == "go")
        .expect("the click journals `go`");
    assert_eq!(
        go.changes,
        ["n", "0", "1", "seen", "0", "1", "word", "zero", "one", "tags", "[]", "[new]"],
        "{go:?}"
    );
    assert_eq!(go.failure, "", "the turn committed");
}
