//! ★ LOCK — `{for each … :} … {else:} … {end}` reaches the browser tier. ★
//!
//! The `{else:}` arm is what a list shows when it is EMPTY — "Nothing to do yet", the first thing
//! anybody sees of a todo app before they add anything. It lowers to an emptiness test around the
//! loop, and that test asked for the list's length by emitting a CALL to the `length` builtin.
//!
//! ★ NOTHING LOWERS THAT CALL. Ordinary LOGOS says `length of xs`, which is `Expr::Length` and has
//! been lowered on every tier since forever; `length(xs)` as a function call is a spelling only the
//! COMPILER ever wrote, so it fell through the wasm backend's builtin match to "builtin not yet
//! lowered" — a message naming neither the page, the loop, nor the `{else:}` that caused it.
//!
//! ★ THE PATTERN, third time in this campaign: a lowering that synthesizes a call nothing expected.
//! The JSON derive hit it (`toJson_` never appears in source, so the loader could not see the
//! demand), the server renderer hit it (`htmlEscape` was demanded and then PRUNED), and this is the
//! same shape one level down — the compiler must write what the LANGUAGE writes, not a second
//! spelling of it that only some tiers happen to understand.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n    a done, which is Bool.\n\n";

/// The same page with `Show html of App.` swapped for `Mount App.` — what a BROWSER runs.
///
/// The content assertions below go through the server renderer, which returns the markup as text
/// and runs on both tiers; the browser tier has no document to print, so it is asserted by
/// COMPILING the mount form. Two questions, two sources, neither standing in for the other.
fn mounting(src: &str) -> String {
    src.replace("Show html of App.", "Mount App.")
}

fn wasm_ok(src: &str) {
    let m = mounting(src);
    // ★ COMPILING IS NOT LOADING. `compile_to_wasm` returns `Ok` for bytes no engine accepts, so
    // `is_ok()` alone would pass on a module that can never instantiate — and a module that does
    // not load is a page that never appears, which is the failure this lock exists to prevent.
    // Validating costs one line and is the difference between "the backend emitted something" and
    // "a browser could run it".
    let bytes =
        compile_to_wasm(&m).unwrap_or_else(|e| panic!("must reach the browser tier: {e:?}\n{m}"));
    let engine = wasmi::Engine::default();
    wasmi::Module::new(&engine, &bytes[..]).unwrap_or_else(|e| {
        panic!("the emitted module must VALIDATE, not merely be emitted: {e}\n{m}")
    });
}

/// The page renders the same on the reference tier and the VM, and compiles for a browser.
fn all_tiers(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "tree-walker vs VM:\n{src}"
    );
    assert!(
        tw.output.contains(expected),
        "expected {expected:?} in:\n{}",
        tw.output
    );
    wasm_ok(src);
}

#[test]
fn an_empty_loop_with_an_else_arm_compiles_for_a_browser() {
    // ★ THE HEADLINE — a scalar list, the simplest form there is.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let names be a new Seq of Text.\n\
       \n\
       View:\n\
      \x20   <ul>{for each n in names:}<li>{n}</li>{else:}<li class=\"empty\">Nothing yet.</li>{end}</ul>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    all_tiers(src, "Nothing yet.");
}

#[test]
fn a_loop_over_records_with_an_else_arm_compiles_for_a_browser() {
    // The shape a real page has: a table of records, empty on first load.
    let src = format!(
        "{TODO}## Component App\n\
         \n\
         State:\n\
        \x20   Let todos be a new Seq of Todo.\n\
         \n\
         View:\n\
        \x20   <ul>{{for each t in todos:}}<li>{{t's text}}</li>{{else:}}<li>Nothing to do.</li>{{end}}</ul>\n\
         \n\
         ## Main\n\
         Show html of App.\n"
    );
    all_tiers(&src, "Nothing to do.");
}

#[test]
fn a_loop_over_records_rendering_a_child_component_compiles_for_a_browser() {
    // ★ EXACTLY THE TODO PAGE: a row component per item, with an empty state beneath it. The child
    // is what makes this worth its own test — the loop body becomes a CALL, so the emptiness test
    // sits beside a different shape of statement than in the cases above.
    let src = format!(
        "{TODO}## Component Row (todo: Todo)\n\
         \n\
         View:\n\
        \x20   <li>{{todo's text}}</li>\n\
         \n\
         ## Component App\n\
         \n\
         State:\n\
        \x20   Let todos be a new Seq of Todo.\n\
         \n\
         View:\n\
        \x20   <ul>{{for each t in todos:}}<Row todo={{t}}/>{{else:}}<li>Nothing to do.</li>{{end}}</ul>\n\
         \n\
         ## Main\n\
         Show html of App.\n"
    );
    all_tiers(&src, "Nothing to do.");
}

#[test]
fn a_non_empty_list_renders_its_rows_and_not_the_else_arm() {
    // ★ THE TEST'S TEETH. Every assertion above would pass against a lowering that emitted the
    // `{else:}` arm ALWAYS — the emptiness test is exactly the thing that could be dropped while
    // making the page compile, and the failure would be an app that says "Nothing to do" beneath
    // a full list.
    let src = format!(
        "{TODO}## Component App\n\
         \n\
         State:\n\
        \x20   Let todos be [a new Todo with id 1 and text \"milk\" and done false].\n\
         \n\
         View:\n\
        \x20   <ul>{{for each t in todos:}}<li>{{t's text}}</li>{{else:}}<li>Nothing to do.</li>{{end}}</ul>\n\
         \n\
         ## Main\n\
         Show html of App.\n"
    );
    let tw = tw_outcome(&src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    assert!(
        tw.output.contains("milk"),
        "the row must render:\n{}",
        tw.output
    );
    assert!(
        !tw.output.contains("Nothing to do"),
        "the else arm must NOT render:\n{}",
        tw.output
    );
    let vm = vm_outcome(&src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "tree-walker vs VM:\n{src}"
    );
    wasm_ok(&src);
}

#[test]
fn a_loop_with_no_else_arm_still_compiles() {
    // The emptiness test is emitted ONLY when there is an arm to run — a loop without one must not
    // pay for a branch it does not have.
    let src = format!(
        "{TODO}## Component App\n\
         \n\
         State:\n\
        \x20   Let todos be a new Seq of Todo.\n\
         \n\
         View:\n\
        \x20   <ul>{{for each t in todos:}}<li>{{t's text}}</li>{{end}}</ul>\n\
         \n\
         ## Main\n\
         Show html of App.\n"
    );
    wasm_ok(&src);
}
