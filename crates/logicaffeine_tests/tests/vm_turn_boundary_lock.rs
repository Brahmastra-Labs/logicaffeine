//! ★ LOCK — a trapping TURN is contained where the tree-walker contains it, on the in-process
//! bytecode tier too. ★
//!
//! A turn is a transaction: an action runs to completion, or the instance's state stays what it
//! was and the page carries on. The page hears a driven turn as an entry of its own — the page
//! host's guarded door on the tree-walker, `BuiltinId::DomDispatchEntry` on the bytecode VM,
//! `dom_page_entry` in the browser — so a turn that fails there rolls its record back, the page's
//! failure door hears it (`When an action fails (e: Text):`), and the program's next statement
//! runs. A failure no handler absorbed is the program's own once nothing is left to run
//! (`driven_turn_failure_lock`).
//!
//! ★ EVERY CASE RUNS ON BOTH TIERS AND THEY MUST AGREE ON ALL THREE: the output, the error, and
//! the page. A door that agreed on the page and not the error — or that contained what the
//! tree-walker lets through — is a second semantics, and the differential contract
//! (`vm_outcome == tw_outcome`) is exactly what it would break.
//!
//! ★ AND IT PINS WHERE CONTAINMENT STOPS. A failure in `## Main` itself is no turn's, and the
//! program ends there.

use logicaffeine_compile::compile::{run_strict_tw, run_strict_vm, tw_outcome, vm_outcome, EngineId};
use logicaffeine_compile::interpreter::{clear_fuel, install_fuel};
use logicaffeine_compile::semantics::dom;

/// What one tier did with a program: every line it showed, how it ended, and the page it left.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
    output: String,
    error: Option<String>,
    page: String,
}

/// The output as its LINES: the VM ends every line with a newline and the tree-walker joins them,
/// and that spelling difference is not what this file is about.
fn run(src: &str, vm: bool) -> Run {
    dom::reset();
    let out = if vm { vm_outcome(src) } else { tw_outcome(src) };
    let output = out.output.lines().collect::<Vec<_>>().join("\n");
    Run { output, error: out.error, page: dom::render() }
}

/// Both tiers, held together on output, error AND page. Returns the reference tier's run.
fn agree(label: &str, src: &str) -> Run {
    let tw = run(src, false);
    let vm = run(src, true);
    assert_eq!(
        vm, tw,
        "`{label}`: the bytecode tier (left) and the tree-walker (right) disagree:\n{src}"
    );
    tw
}

/// The trap every case uses: reading past the end of a one-element list. A RUNTIME failure — the
/// index is only known when it runs — so it fails the turn rather than the compile.
const OUT_OF_BOUNDS: &str = "Index 9 out of bounds";

// ─── the turn is caught ──────────────────────────────────────────────────────────────────────

#[test]
fn the_page_still_answers_after_a_trapping_turn_on_both_tiers() {
    let src = "## Component App\nState:\n    Let n be 0.\n    Let boom be 0.\nActions:\n\
               \x20   To bad:\n        Set n to 5.\n        Set boom to item 9 of [1].\n\
               \x20   To good:\n        Set n to 3.\nView:\n\
               \x20   <main><p id=\"n\">{n}</p><button id=\"b\" on-click={bad}>x</button>\
               <button id=\"g\" on-click={good}>y</button></main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nClick \"#g\".\n";
    let r = agree("recover", src);
    assert_eq!(
        r.error.as_deref(),
        Some(OUT_OF_BOUNDS),
        "a trapping turn no handler absorbs is the program's failure once nothing is left to run"
    );
    assert!(
        r.page.contains("<p id=\"n\">3</p>"),
        "the second click must run and commit 3 — not the 5 the trapping turn never committed: {}",
        r.page
    );
}

#[test]
fn a_trapping_turn_writes_nothing_back() {
    // A scalar, a Text and a list push, all BEFORE the trap: a rollback that restored scalars and
    // forgot the row it pushed is a page holding a value nobody committed.
    let src = "## Component App\nState:\n    Let n be 0.\n    Let note be \"\".\n\
               \x20   Let items be [7].\n    Let boom be 0.\nActions:\n\
               \x20   To bad:\n        Set n to 5.\n        Set note to \"w\".\n\
               \x20       Push 1 to items.\n        Set boom to item 9 of [1].\nView:\n\
               \x20   <main><p id=\"n\">{n}</p><p id=\"t\">{note}</p><p id=\"c\">{length of items}</p>\
               <button id=\"b\" on-click={bad}>x</button></main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nClick \"#b\".\n";
    let r = agree("rollback", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert!(
        r.page.contains("<p id=\"n\">0</p><p id=\"t\"></p><p id=\"c\">1</p>"),
        "two trapping turns must leave every field as it was: {}",
        r.page
    );
}

#[test]
fn the_failure_handler_hears_what_went_wrong() {
    let src = "## Component App\nState:\n    Let n be 0.\n    Let why be \"\".\n    Let boom be 0.\n\
               Actions:\n    To bad:\n        Set n to 5.\n        Set boom to item 9 of [1].\n\
               \x20   When an action fails (e: Text):\n        Set why to e.\nView:\n\
               \x20   <main><p id=\"n\">{n}</p><p id=\"w\">{why}</p>\
               <button id=\"b\" on-click={bad}>x</button></main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\n";
    let r = agree("failure handler", src);
    assert_eq!(r.error, None);
    assert!(
        r.page.contains(&format!("<p id=\"n\">0</p><p id=\"w\">{OUT_OF_BOUNDS}</p>")),
        "the handler sees the failure's own words, against the record the turn started from: {}",
        r.page
    );
}

#[test]
fn lines_shown_before_the_trap_stay_shown_and_the_program_carries_on() {
    let src = "## Component App\nState:\n    Let boom be 0.\nActions:\n\
               \x20   To bad:\n        Show \"before\".\n        Set boom to item 9 of [1].\n\
               \x20       Show \"never\".\nView:\n\
               \x20   <main><button id=\"b\" on-click={bad}>x</button></main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nShow \"after\".\n";
    let r = agree("output", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert_eq!(r.output, "before\nafter", "output is not rolled back, and nothing after the trap runs");
}

#[test]
fn a_failure_handler_that_fails_is_swallowed_and_commits_nothing() {
    // The reference tier runs the handler and keeps its writes only if it finishes. A handler
    // that fails absorbs nothing, and its own failure never replaces the one being reported, so
    // the turn's failure is the program's. And the page still answers the next click.
    let src = "## Component App\nState:\n    Let n be 0.\n    Let why be \"\".\n    Let boom be 0.\n\
               Actions:\n    To bad:\n        Set n to 5.\n        Set boom to item 9 of [1].\n\
               \x20   To good:\n        Set n to 3.\n\
               \x20   When an action fails (e: Text):\n        Set why to e.\n\
               \x20       Set boom to item 8 of [1].\nView:\n\
               \x20   <main><p id=\"n\">{n}</p><p id=\"w\">{why}</p>\
               <button id=\"b\" on-click={bad}>x</button><button id=\"g\" on-click={good}>y</button>\
               </main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nClick \"#g\".\nShow \"after\".\n";
    let r = agree("failing handler", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert_eq!(r.output, "after");
    assert!(
        r.page.contains("<p id=\"n\">3</p><p id=\"w\"></p>"),
        "the handler's own write must not commit, and the next turn must run: {}",
        r.page
    );
}

#[test]
fn a_store_action_that_traps_fails_the_turn_that_called_it() {
    // A store call inside an action is NOT a turn of its own: its failure is the calling turn's
    // failure, so the caller's write goes too.
    let src = "## Store Counter\nState:\n    Let n be 0.\n    Let boom be 0.\nActions:\n\
               \x20   To bump:\n        Set n to n + 1.\n        Set boom to item 9 of [1].\n\n\
               ## Component App\nUse Counter.\nState:\n    Let m be 0.\nActions:\n\
               \x20   To go:\n        Set m to 7.\n        Call Counter's bump.\nView:\n\
               \x20   <p id=\"n\">{Counter's n}/{m}</p>\n\
               \x20   <button id=\"b\" on-click={go}>+</button>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nShow \"after\".\n";
    let r = agree("store trap", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert_eq!(r.output, "after");
    assert!(r.page.contains("<p id=\"n\">0/0</p>"), "neither write commits: {}", r.page);
}

#[test]
fn a_store_write_that_committed_before_the_trap_stays_committed() {
    // The store's own call finished and committed; the calling turn trapped after it. The
    // reference tier rolls back only the CALLER's record — the store keeps its write.
    let src = "## Store Counter\nState:\n    Let n be 0.\nActions:\n\
               \x20   To bump:\n        Set n to n + 1.\n\n\
               ## Component App\nUse Counter.\nState:\n    Let m be 0.\n    Let boom be 0.\n\
               Actions:\n    To go:\n        Set m to 7.\n        Call Counter's bump.\n\
               \x20       Set boom to item 9 of [1].\nView:\n\
               \x20   <p id=\"n\">{Counter's n}/{m}</p>\n\
               \x20   <button id=\"b\" on-click={go}>+</button>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nShow \"after\".\n";
    let r = agree("store committed", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert!(r.page.contains("<p id=\"n\">1/0</p>"), "the store keeps its write: {}", r.page);
}

#[test]
fn a_turn_driven_from_inside_a_turn_is_caught_at_its_own_boundary() {
    // `Click` inside an action delivers a turn of its own. Its failure is contained THERE, so the
    // outer turn carries on and commits — it is not the outer turn's failure — and, unabsorbed,
    // it is the program's at the end. The VM and the browser (`dom_page_entry`) deliver it so; the
    // tree-walker's page host refuses a page driving itself ("the page's own code cannot drive
    // the page it is running in"), so the tiers disagree here until that is settled.
    let src = "## Component App\nState:\n    Let n be 0.\n    Let m be 0.\n    Let boom be 0.\n\
               Actions:\n    To outer:\n        Set m to 1.\n        Click \"#i\".\n\
               \x20       Set m to 2.\n    To inner:\n        Set n to 9.\n\
               \x20       Set boom to item 9 of [1].\nView:\n\
               \x20   <main><p id=\"n\">{n}/{m}</p><button id=\"o\" on-click={outer}>o</button>\
               <button id=\"i\" on-click={inner}>i</button></main>\n\n\
               ## Main\nMount App.\nClick \"#o\".\nShow \"after\".\n";
    let r = agree("nested turn", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert!(r.page.contains("<p id=\"n\">0/2</p>"), "inner rolled back, outer committed: {}", r.page);
}

#[test]
fn a_trapping_turn_is_caught_every_time() {
    let src = "## Component App\nState:\n    Let n be 0.\n    Let tries be 0.\n    Let boom be 0.\n\
               Actions:\n    To bad:\n        Set tries to tries + 1.\n\
               \x20       Set boom to item 9 of [1].\n    To good:\n        Set n to n + 1.\n\
               View:\n    <main><p id=\"n\">{n}/{tries}</p><button id=\"b\" on-click={bad}>x</button>\
               <button id=\"g\" on-click={good}>y</button></main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nClick \"#g\".\nClick \"#b\".\nClick \"#b\".\n\
               Click \"#g\".\n";
    let r = agree("repeated", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert!(r.page.contains("<p id=\"n\">2/0</p>"), "every trap caught, every good turn kept: {}", r.page);
}

#[test]
fn a_failed_turn_is_answered_on_the_instance_it_ran_against() {
    // Two placements of one component, one record each. The second click on `b` traps; its
    // failure is `b`'s alone — the rollback, the handler's message and the repaint all land on
    // `b`'s record, and `a` never hears of it. A failed-turn path that ran against the root
    // instance, or the first one, would put the message on the wrong row or on none.
    let src = "## Component Row (rid: Text)\nState:\n    Let n be 0.\n    Let why be \"\".\n\
               \x20   Let boom be 0.\nActions:\n    To bump:\n        Set n to n + 1.\n\
               \x20       If n is 2:\n            Set boom to item 9 of [1].\n\
               \x20   When an action fails (e: Text):\n        Set why to e.\nView:\n\
               \x20   <li><button id={rid} on-click={bump}>{n}</button><em>{why}</em></li>\n\n\
               ## Component App\nView:\n    <ul><Row rid=\"a\"/><Row rid=\"b\"/></ul>\n\n\
               ## Main\nMount App.\nClick \"#a\".\nClick \"#b\".\nClick \"#b\".\n";
    let r = agree("instances", src);
    assert_eq!(r.error, None);
    assert!(
        r.page.contains(&format!(
            "<li><button id=\"a\">1</button><em></em></li>\
             <li><button id=\"b\">1</button><em>{OUT_OF_BOUNDS}</em></li>"
        )),
        "the failure is `b`'s: its count rolls back to 1, only it hears the message: {}",
        r.page
    );
}

#[test]
fn a_step_budgeted_run_recovers_the_same_way() {
    // A budgeted run takes the VM's STEPPED loop (`largo run --max-steps`, the MCP step budget),
    // a separate monomorph of the dispatch loop. The door must not live in only one of them.
    let src = "## Component App\nState:\n    Let n be 0.\n    Let boom be 0.\nActions:\n\
               \x20   To bad:\n        Set n to 5.\n        Set boom to item 9 of [1].\n\
               \x20   To good:\n        Set n to 3.\nView:\n\
               \x20   <main><p id=\"n\">{n}</p><button id=\"b\" on-click={bad}>x</button>\
               <button id=\"g\" on-click={good}>y</button></main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nClick \"#g\".\n";
    let budgeted = |vm: bool| {
        dom::reset();
        let fuel = install_fuel(50_000_000);
        let out = if vm { run_strict_vm(src) } else { run_strict_tw(src) };
        clear_fuel();
        assert!(!fuel.exhausted.get(), "the budget is generous: it must not be what stops the run");
        (out, dom::render())
    };
    let (tw, tw_page) = budgeted(false);
    let (vm, vm_page) = budgeted(true);
    assert_eq!(vm.engine_ran, Some(EngineId::Vm), "the strict run must be the VM's own");
    assert_eq!((vm.error.clone(), vm_page.clone()), (tw.error, tw_page));
    assert_eq!(vm.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert!(vm_page.contains("<p id=\"n\">3</p>"), "{vm_page}");
}

// ─── where containment stops ─────────────────────────────────────────────────────────────────

#[test]
fn a_failure_outside_any_turn_still_ends_the_program() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        Set n to 5.\n\
               View:\n    <main><p id=\"n\">{n}</p><button id=\"b\" on-click={go}>x</button></main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nShow item 9 of [1].\nShow \"after\".\n";
    let r = agree("main trap", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS), "Main's failure is the program's");
    assert_eq!(r.output, "");
}

#[test]
fn a_view_that_cannot_paint_the_committed_state_fails_the_entry_and_the_program_runs_on() {
    // The turn commits `n = 5`, and only then does the paint fail. The paint is part of the
    // entry the page heard, so the door contains it like any other failure in the entry: the
    // program runs on, and the failure no handler absorbed is the program's at the end.
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        Set n to 5.\n\
               View:\n    <main><p id=\"n\">{item (n + 1) of [1]}</p>\
               <button id=\"b\" on-click={go}>x</button></main>\n\n\
               ## Main\nMount App.\nClick \"#b\".\nShow \"after\".\n";
    let r = agree("paint failure", src);
    assert_eq!(r.error.as_deref(), Some("Index 6 out of bounds"), "a paint failure is the program's");
    assert_eq!(r.output, "after", "the statement after the failed entry runs");
}

#[test]
fn a_page_load_handler_that_traps_still_ends_the_program() {
    // `When the page loads:` runs as the page mounts, not as a delivered turn — so its failure is
    // the program's, and the program ends there, as the browser's page main does when its mount
    // traps. The VM agrees; the tree-walker's page host carries `Mount` through its guarded door
    // and runs on, so the tiers disagree here until that is settled.
    let src = "## Component App\nState:\n    Let n be 0.\n    Let boom be 0.\nActions:\n\
               \x20   When the page loads:\n        Set n to 5.\n        Set boom to item 9 of [1].\n\
               View:\n    <p id=\"n\">{n}</p>\n\n## Main\nMount App.\nShow \"after\".\n";
    let r = agree("page load", src);
    assert_eq!(r.error.as_deref(), Some(OUT_OF_BOUNDS));
    assert_eq!(r.output, "");
}

// ─── the sweep ───────────────────────────────────────────────────────────────────────────────

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const TRAP: &str = "        Set boom to item 9 of [1].\n";

/// One page from one draw: an action writing a mix of scalars, Text and list rows — some inside a
/// branch that really runs — with the trap at a drawn position when `armed`; a second action that
/// always commits; and, on some draws, a failure handler. Main clicks the first, then the second.
fn page(seed: u64, armed: bool, clicked: bool) -> (String, bool) {
    let mut rng = Rng(seed);
    let writes = 1 + rng.below(3);
    let trap_at = rng.below(writes + 1);
    let handled = rng.below(2) == 0;

    let mut state = String::from(
        "    Let boom be 0.\n    Let note be \"\".\n    Let items be [7].\n    Let seen be 0.\n    Let why be \"\".\n",
    );
    let mut body = String::new();
    for i in 0..writes {
        state.push_str(&format!("    Let f{i} be 0.\n"));
        if armed && i == trap_at {
            body.push_str(TRAP);
        }
        match rng.below(3) {
            0 => body.push_str(&format!("        Set f{i} to {}.\n", 1 + i)),
            1 => body.push_str(&format!("        Set note to \"w{i}\".\n")),
            _ => body.push_str(&format!("        Push {} to items.\n", 1 + i)),
        }
        if rng.below(3) == 0 {
            body.push_str(&format!("        If f{i} is at least 0:\n            Set f{i} to f{i} + 1.\n"));
        }
    }
    if armed && trap_at == writes {
        body.push_str(TRAP);
    }
    let handler = if handled { "    When an action fails (e: Text):\n        Set why to e.\n" } else { "" };
    let mut view = String::from("    <main>");
    for i in 0..writes {
        view.push_str(&format!("<p id=\"f{i}\">{{f{i}}}</p>"));
    }
    view.push_str("<p id=\"t\">{note}</p><p id=\"c\">{length of items}</p><p id=\"s\">{seen}</p>");
    view.push_str("<p id=\"w\">{why}</p><button id=\"go\" on-click={act}>x</button>");
    view.push_str("<button id=\"ok\" on-click={good}>y</button></main>\n");
    let go = if clicked { "Click \"#go\".\n" } else { "" };
    (
        format!(
            "## Component App\nState:\n{state}Actions:\n    To act:\n{body}    To good:\n        Set seen to seen + 1.\n{handler}View:\n{view}\n## Main\nMount App.\n{go}Click \"#ok\".\n"
        ),
        handled,
    )
}

fn sweep(seed: u64, cases: usize) -> usize {
    let mut rng = Rng(seed);
    let mut checked = 0;
    for _ in 0..cases {
        let draw = rng.next();
        let (armed, handled) = page(draw, true, true);
        let (disarmed, _) = page(draw, false, true);
        let (untouched, _) = page(draw, true, false);

        let after_trap = agree("armed", &armed);
        let after_clean = agree("disarmed", &disarmed);
        let before = agree("untouched", &untouched);
        assert_eq!(
            after_trap.error.as_deref(),
            (!handled).then_some(OUT_OF_BOUNDS),
            "a trapping turn is the program's failure exactly when no handler absorbs it:\n{armed}"
        );
        assert!(
            after_trap.page.contains("<p id=\"s\">1</p>"),
            "the page must answer the click after the trap:\n{armed}\n{}",
            after_trap.page
        );
        // Everything the trapping turn wrote is gone — the page is the one that never clicked
        // `#go`, except for what the failure handler was told.
        let expected = if handled {
            before.page.replace("<p id=\"w\"></p>", &format!("<p id=\"w\">{OUT_OF_BOUNDS}</p>"))
        } else {
            before.page.clone()
        };
        assert_eq!(after_trap.page, expected, "a TRAPPING turn left state behind:\n{armed}");
        // The teeth: without the trap the same turn really does change the page, so "unchanged"
        // cannot pass by the turn having done nothing.
        assert_ne!(after_clean.page, before.page, "the DISARMED turn changed nothing:\n{disarmed}");
        checked += 1;
    }
    checked
}

#[test]
fn a_trapping_turn_commits_nothing_on_either_tier_over_a_sweep() {
    assert_eq!(sweep(0x5DEE_CE66_D1CE_4E5B, 24), 24);
}
