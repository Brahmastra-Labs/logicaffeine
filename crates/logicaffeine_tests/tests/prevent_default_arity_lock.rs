//! ★ LOCK — `on-submit.prevent` runs on the BYTECODE tier, not just the reference one. ★
//!
//! `.prevent` is what stops a handled form from ALSO posting itself natively — the duplicate
//! submission every form library exists to avoid. It lowers to `preventDefault(element, event)`,
//! two arguments, and the builtin's implementation takes two. Its ARITY TABLE said one.
//!
//! ★ SO THE REFERENCE TIER WAS FINE AND EVERY OTHER TIER TRAPPED. The tree-walker reaches the
//! modifier through its own listener table and never calls the lowered builtin, so
//! `progressive_form_lock` — which drives that tier — passed throughout. The VM refused with
//! "preventDefault() takes exactly 1 argument", and the WebAssembly module, which is what a
//! visitor actually runs, trapped with `UnreachableCodeReached` the moment the page mounted.
//! A page with a form could not load in a browser at all.
//!
//! ★ THE SHAPE OF THE MISTAKE, worth more than the fix: a builtin has FIVE registration sites and
//! the arity table is the one with no compiler check behind it — an exhaustive `match` catches a
//! missing name, and a fall-through default catches nothing. That is why this lock asserts the
//! arity DIRECTLY as well as running the page: the direct assertion says what is wrong in one
//! line, and the page says it matters.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::builtins::{check_arity, BuiltinId};

/// A page whose form handles its own submit and suppresses the native post.
const PAGE: &str = "## Component App\n\
   \n\
   State:\n\
  \x20   Let saved be false.\n\
   \n\
   Actions:\n\
  \x20   To save:\n\
  \x20       Set saved to true.\n\
   \n\
   View:\n\
  \x20   <form id=\"f\" on-submit.prevent={save}>\n\
  \x20     <button type=\"submit\">Go</button>\n\
  \x20   </form>\n\
   \n\
   ## Main\n\
   Mount App.\n";

#[test]
fn prevent_default_takes_the_element_and_the_event() {
    // ★ THE ONE-LINE STATEMENT OF THE BUG. The implementation reads two arguments; the table said
    // one, so every caller of the lowering was refused before it ran.
    assert!(
        check_arity(BuiltinId::DomPreventDefault, 2).is_ok(),
        "preventDefault(element, event) takes the element AND the event: {:?}",
        check_arity(BuiltinId::DomPreventDefault, 2)
    );
    // And it is not silently generous: one argument is still wrong.
    assert!(
        check_arity(BuiltinId::DomPreventDefault, 1).is_err(),
        "one argument is not enough"
    );
}

#[test]
fn a_page_with_a_prevented_form_runs_on_the_vm() {
    // The tier that caught it, and the one whose message named the cause.
    let out = vm_outcome(PAGE);
    assert_eq!(
        out.error, None,
        "the VM must run a page whose form prevents its default"
    );
}

#[test]
fn a_page_with_a_prevented_form_runs_on_the_reference_tier() {
    // The tier that was always green — kept beside the others so the DIFFERENCE is what the file
    // records, not just the failure.
    let out = tw_outcome(PAGE);
    assert_eq!(out.error, None, "the tree-walker must run it too");
}

#[test]
fn a_page_with_a_prevented_form_produces_a_module_a_browser_can_load() {
    // ★ COMPILING IS NOT ENOUGH, and this file learned that twice. The arity fix made the page
    // compile and the module still failed to VALIDATE — the effect's destination register had no
    // declared kind, so it defaulted to an i64 local while the lowering stored the i32 `Nothing`
    // handle into it. A module that does not validate never instantiates: the page simply does not
    // load, with nothing anywhere naming the `.prevent` that caused it.
    let module =
        compile_to_wasm(PAGE).expect("a page with a prevented form must reach the browser tier");
    let engine = wasmi::Engine::default();
    wasmi::Module::new(&engine, &module[..])
        .unwrap_or_else(|e| panic!("the module must VALIDATE, or no browser will load it: {e}"));
}

#[test]
fn every_dom_builtin_that_takes_an_event_declares_two_arguments() {
    // ★ THE NEIGHBOURS, checked so the next one added is not the next one forgotten. Each of these
    // takes an element plus one more thing; a default of 1 would refuse every call.
    for (b, name) in [
        (BuiltinId::DomPreventDefault, "preventDefault"),
        (BuiltinId::DomSetText, "setText"),
        (BuiltinId::DomSetValue, "setValue"),
        (BuiltinId::DomSetChecked, "setChecked"),
        (BuiltinId::DomAppendChild, "appendChild"),
    ] {
        assert!(
            check_arity(b, 2).is_ok(),
            "{name} takes an element and one more argument: {:?}",
            check_arity(b, 2)
        );
    }
}
