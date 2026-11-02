//! ★ LOCK — a `bind` the browser cannot honour is REFUSED, not quietly turned into a text write. ★
//!
//! `bound_input_kind` decides what a `bind` writes, and it already holds the right instinct in its
//! own doc comment: a computed `type=` is `Unknown` and *"refused rather than guessed, since
//! guessing `value` renders a checkbox the browser draws unchecked while the page believes it is
//! checked."*
//!
//! ★ IT IS HANDED THE ATTRIBUTES AND NEVER THE TAG, so two elements it cannot honour fall through
//! to `Value` and get a text `setValue`:
//!
//! ```text
//! <input type="file" bind={f}>      →  BoundInput::Value  (no `type` arm matches "file")
//! <select multiple bind={s}>        →  BoundInput::Value  (no `type` attribute at all)
//! ```
//!
//! Both are the silent wrong answer that enum exists to prevent, and neither is a near miss:
//!
//! * A browser **will not let a page set a file input's value** — it is a deliberate restriction,
//!   because a page that could would be a page that can read any file on your disk by naming it.
//!   So the write does nothing, silently, forever.
//! * A `<select multiple>`'s value is the *list* of chosen options. Writing one text into it picks
//!   at most one, and reading it back gives at most one — so a visitor's other selections vanish
//!   with no error anywhere.
//!
//! In both cases the author has been told their binding was understood. That is the failure this
//! file turns into a refusal that names the element and says what to write instead.
//!
//! ★ THE FIX IS THE QUESTION, NOT THE ANSWER: `bound_input_kind` is given the TAG, and
//! `BoundInput::refusal()` is TOTAL — a new variant does not compile until somebody decides whether
//! it lowers or is refused by name. Silently absent is not one of the options, which is the same
//! discipline `action_verb_tier_lock` holds over `UiStmt`.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

fn page(markup: &str) -> String {
    format!(
        "## Component App\nState:\n\x20   Let f be \"\".\nView:\n\x20   {markup}\n\n\
         ## Main\nMount App.\n"
    )
}

/// The refusal both driving tiers give, asserted to be the SAME message on each.
fn refusal(subject: &str, markup: &str) -> String {
    dom::reset();
    let tw = tw_outcome(&page(markup))
        .error
        .unwrap_or_else(|| panic!("`{subject}` must be refused on the tree-walker:\n{markup}"));
    dom::reset();
    let vm = vm_outcome(&page(markup))
        .error
        .unwrap_or_else(|| panic!("`{subject}` must be refused on the VM too:\n{markup}"));
    assert!(
        vm.contains(&tw) || tw.contains(&vm),
        "two tiers refusing the same thing in different words send whoever reads one looking for \
         the other:\n  tw: {tw}\n  vm: {vm}"
    );
    tw
}

fn runs(subject: &str, markup: &str) {
    dom::reset();
    let out = tw_outcome(&page(markup));
    assert_eq!(out.error, None, "`{subject}` must still work:\n{markup}");
}

#[test]
fn a_bind_on_a_file_input_is_refused() {
    let err = refusal("a file input", "<input type=\"file\" bind={f}>");
    let lower = err.to_lowercase();
    assert!(
        lower.contains("file"),
        "the refusal must name the element: {err}"
    );
    assert!(
        lower.contains("cannot") || lower.contains("will not") || lower.contains("refus"),
        "and say the browser will not allow it: {err}"
    );
}

#[test]
fn a_bind_on_a_multiple_select_is_refused() {
    let err = refusal(
        "a multiple select",
        "<select multiple bind={f}><option>a</option></select>",
    );
    let lower = err.to_lowercase();
    assert!(
        lower.contains("multiple") || lower.contains("select"),
        "the refusal must name the element: {err}"
    );
}

#[test]
fn a_single_select_still_binds() {
    // ★ THE NARROWNESS GUARD. A plain `<select>` has exactly one chosen option and its value IS a
    // text, so it is the ordinary case — a refusal that swept up every `<select>` would break the
    // most common form control there is after a text box.
    runs(
        "a single select",
        "<select bind={f}><option>a</option></select>",
    );
}

#[test]
fn a_textarea_still_binds() {
    // It carries no `type` either, which is exactly why a tag-blind check called it `Value` — and
    // for a textarea that answer happened to be right. The fix must not lose it.
    runs("a textarea", "<textarea bind={f}></textarea>");
}

#[test]
fn every_ordinary_input_still_binds() {
    runs("a bare input", "<input bind={f}>");
    runs("a text input", "<input type=\"text\" bind={f}>");
    runs("an email input", "<input type=\"email\" bind={f}>");
    runs("a number input", "<input type=\"number\" bind={f}>");
}

#[test]
fn a_checkbox_and_a_radio_still_bind_their_checked_state() {
    runs("a checkbox", "<input type=\"checkbox\" bind={f}>");
    runs("a radio", "<input type=\"radio\" bind={f}>");
}

#[test]
fn a_computed_type_is_still_refused() {
    // The refusal that already existed keeps working — this file adds two, it does not replace one.
    let err = refusal("a computed type", "<input type={f} bind={f}>");
    assert!(err.to_lowercase().contains("computed"), "{err}");
}
