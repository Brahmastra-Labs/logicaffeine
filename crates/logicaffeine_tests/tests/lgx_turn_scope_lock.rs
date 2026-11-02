//! ★ LOCK — a turn's OWN compiled function sees everything the reference tier's turn does: every
//! declared PROP, not only state, and the `When <field> changes:` watcher on every write, not only
//! ones a hand-rolled shortcut remembered to run through it. ★
//!
//! Both classes were found the same way: a component with an `Actions:`/event handler that reads a
//! PROP (never a state field) traps with `Undefined variable: <prop>` on the bytecode VM — every
//! OTHER turn shape read state fields in via `state_read`, but nothing ever bound a prop the same
//! way, because a render function receives a prop as a plain parameter and never needed to. A bound
//! input's own write-back (`__bind_<Component>_<field>`) is a turn too, but was hand-rolled instead
//! of built through `turn_body_with` — the one place the `When <field> changes:` watcher lives — so
//! typing into a bound field could never wake a sentence watching the very field it just wrote.
//!
//! ★ VM-ONLY, DELIBERATELY. The reference tree-walker has its OWN, separate defect reading a prop
//! inside a turn (`Undefined variable: <prop>` there too, from different code — `interpreter.rs`,
//! not this lowering) — tracked separately, not this file's fix to make. Every test here drives
//! `vm_outcome` alone; `lgx_vm_parity_lock.rs` is where a construct earns a tree-walker-agrees test,
//! once the reference tier's own gap closes.

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::semantics::dom;

fn vm_output(src: &str) -> String {
    dom::reset();
    let outcome = vm_outcome(src);
    assert!(
        outcome.error.is_none(),
        "must not trap: {:?}\n{src}",
        outcome.error
    );
    outcome.output
}

#[test]
fn a_component_placed_once_reads_its_own_prop_in_an_appears_handler() {
    let src = "\
## Component Row (label: Text)
Actions:
    When this component appears:
        Show \"in {label}\".
View:
    <li>{label}</li>

## Component App
View:
    <main><ul><Row label=\"only\"/></ul></main>

## Main
Mount App.
";
    assert_eq!(
        vm_output(src).lines().collect::<Vec<_>>(),
        ["in only"],
        "a single placed instance must read its own prop inside its Appears turn"
    );
}

#[test]
fn a_component_placed_by_a_loop_reads_its_own_prop_in_an_appears_handler() {
    let src = "\
## Component Row (label: Text)
Actions:
    When this component appears:
        Show \"in {label}\".
View:
    <li>{label}</li>

## Component App
View:
    <main><ul>{for each r in [\"a\", \"b\"]:}<Row label=\"{r}\"/>{end}</ul></main>

## Main
Mount App.
";
    assert_eq!(
        vm_output(src).lines().collect::<Vec<_>>(),
        ["in a", "in b"],
        "every loop-placed instance must read its OWN prop, not another instance's or none at all"
    );
}

#[test]
fn a_prop_read_in_an_action_agrees_with_a_prop_read_in_the_view() {
    // Not just Appears: ANY turn (an ordinary action here) must see a declared prop too.
    let src = "\
## Component Row (label: Text)
Actions:
    To announce:
        Show \"row {label}\".
View:
    <li>{label}</li>
    <button id=\"go\" on-click={announce}>go</button>

## Component App
View:
    <main><ul><Row label=\"x\"/></ul></main>

## Main
Mount App.
Click \"#go\".
";
    assert_eq!(vm_output(src).lines().collect::<Vec<_>>(), ["row x"]);
}

#[test]
fn typing_into_a_bound_field_wakes_a_sentence_watching_that_same_field() {
    let src = "\
## Component App
State:
    Let query be \"\".
    Let searches be 0.
Actions:
    When query changes:
        Set searches to searches + 1.
        Show \"searches: {searches}\".
View:
    <input id=\"q\" bind={query}>
    <p id=\"n\">{searches}</p>

## Main
Mount App.
Type \"tea\" into \"#q\".
";
    assert_eq!(
        vm_output(src).lines().collect::<Vec<_>>(),
        ["searches: 1"],
        "the bound field's own write-back must run through the SAME watcher mechanism every other \
         write does — `lower_bind_writeback` hand-rolled its body and skipped it entirely"
    );
}
