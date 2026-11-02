//! ★ LOCK — `Inspect` over an `Option of <record>` matches `When Some (v)`. ★
//!
//! LOGOS encodes an Option TRANSPARENTLY: `some x` IS x, and `none` IS Nothing. That is what
//! makes `Option of Int` work — an Int is not a variant of anything, so a `Some` arm matches it
//! by falling through to "any value that is not Nothing".
//!
//! A RECORD took a different path. Both tiers checked a struct against the arm's variant NAME
//! first (`Response` vs `Some`), found no match, and moved on — never reaching the transparent
//! case, because the struct branch had already claimed the value. So `Option of Int` worked and
//! `Option of <any record>` could not be inspected AT ALL:
//!
//! ```text
//! Inspect has no arm for the value and no Otherwise (matches must be exhaustive)
//! ```
//!
//! The asymmetry is invisible until someone writes the first function returning an Option of a
//! record — which is the plan's own middleware signature, `-> Option of Response`. It was found
//! by building that, and it is pinned HERE in plain LOGOS rather than beside the router, because
//! nothing about it is about the web.
//!
//! Binding is the second half and fails differently: a real record arm binds a FIELD by name, a
//! `Some` arm binds the WHOLE value. A fix that matched but bound nothing would give an arm whose
//! variable is quietly empty, which is worse than the loud error it replaced.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers, same answer, no error.
fn both(src: &str, want: &[&str]) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it:\n{src}");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        want,
        "tree-walker:\n{src}"
    );
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), want, "VM:\n{src}");
}

const BOX: &str = "## A Box has:\n    a v, which is Text.\n\n";

#[test]
fn a_some_arm_matches_a_record_payload_and_binds_the_whole_record() {
    let src = format!(
        "{BOX}## To f () -> Option of Box:\n    Return some(a new Box with v \"hi\").\n\n\
         ## Main\nInspect f():\n    When Some (b):\n        Show b's v.\n    When None:\n        Show \"none\".\n"
    );
    both(&src, &["hi"]);
}

#[test]
fn a_none_arm_still_matches_when_the_option_is_empty() {
    let src = format!(
        "{BOX}## To f () -> Option of Box:\n    Return none.\n\n\
         ## Main\nInspect f():\n    When Some (b):\n        Show b's v.\n    When None:\n        Show \"none\".\n"
    );
    both(&src, &["none"]);
}

#[test]
fn the_same_shape_works_for_a_scalar_payload() {
    // The case that always worked, kept beside the one that did not — so the two can never
    // diverge again without something going red.
    let src = "## To f () -> Option of Int:\n    Return some(7).\n\n\
        ## Main\nInspect f():\n    When Some (v):\n        Show v.\n    When None:\n        Show \"none\".\n";
    both(src, &["7"]);
}

#[test]
fn a_real_record_arm_still_binds_by_field_name() {
    // The half a careless fix breaks: an ordinary record arm must keep binding its FIELD. If
    // `Some` matching had been made to swallow every struct, this would bind the whole shape.
    let src = "## A Circle has:\n    a radius, which is Int.\n\n\
        ## Main\nLet c be a new Circle with radius 5.\n\
        Inspect c:\n    When Circle (radius):\n        Show radius.\n    Otherwise:\n        Show \"other\".\n";
    both(src, &["5"]);
}

#[test]
fn a_record_arm_naming_a_field_that_does_not_exist_fails_the_same_way_on_both_tiers() {
    // ★ The divergence the first version of this fix INTRODUCED, kept as a guard.
    //
    // A `Some` arm binds the whole value; a record arm binds a field. Both reach the VM's
    // `BindArm` looking alike, and the first fix told them apart by BINDING INDEX — "a field miss
    // at position 0 means Some". That also fires for `When Circle (r)`, a real record arm naming a
    // field that does not exist: the tree-walker errors on the unbound name, and the VM silently
    // bound the whole Circle and printed it. A marker no identifier can spell tells the two cases
    // apart exactly, instead of guessing from shape.
    let src = "## A Circle has:\n    a radius, which is Int.\n\n\
        ## Main\nLet c be a new Circle with radius 5.\n\
        Inspect c:\n    When Circle (r):\n        Show r.\n    Otherwise:\n        Show \"other\".\n";
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert!(
        tw.error.is_some(),
        "naming a field the record does not have must FAIL, not bind something else"
    );
    assert_eq!(
        tw.error, vm.error,
        "and it must fail the same way on both tiers"
    );
}

#[test]
fn an_option_of_record_round_trips_through_a_function_boundary() {
    // The shape middleware actually uses: one function answers an Option of a record, another
    // inspects it. Crossing the boundary is where a value's encoding is most easily lost.
    let src = format!(
        "{BOX}## To wrap (t: Text) -> Option of Box:\n\
         \x20   If t is equal to \"\":\n        Return none.\n\
         \x20   Return some(a new Box with v t).\n\n\
         ## To unwrap (o: Option of Box) -> Text:\n\
         \x20   Inspect o:\n        When Some (b):\n            Return b's v.\n        When None:\n            Return \"empty\".\n\n\
         ## Main\nShow unwrap(wrap(\"here\")).\nShow unwrap(wrap(\"\")).\n"
    );
    both(&src, &["here", "empty"]);
}

// ---------------------------------------------------------------------------------------------
// ★ `x is none` — the plainest way to ask whether an optional value is there. ★
// ---------------------------------------------------------------------------------------------

const PICK: &str = "## To pick (yes: Bool) -> Option of Text:\n\
    \x20   If yes:\n\
    \x20       Return some(\"here\").\n\
    \x20   Return none.\n\n";

#[test]
fn a_value_that_is_absent_answers_is_none() {
    // ★ IT DID NOT PARSE AT ALL. The only spellings were `is equal to none` and an `Inspect` with
    // a `When None:` arm, and what an author got for the natural sentence was "This construction
    // needs the word ':' here to complete its shape" — an error about PUNCTUATION for a phrase
    // that was never a form, which sends the reader hunting for a colon that is not missing.
    // This project's own design docs write `currentUser(request) is none`, so the sentence
    // everybody reaches for was the one sentence the language refused.
    both(
        &format!("{PICK}## Main\nLet t be pick(false).\nIf t is none:\n    Show \"absent\".\n"),
        &["absent"],
    );
}

#[test]
fn a_value_that_is_present_is_not_none() {
    both(
        &format!(
            "{PICK}## Main\nLet t be pick(true).\nIf t is not none:\n    Show \"here: \" + t.\n"
        ),
        &["here: here"],
    );
}

#[test]
fn the_question_can_be_asked_of_a_call_directly() {
    // The plan's middleware guard asks it of a CALL, not a bound name — `currentUser(r) is none` —
    // so the predicate has to attach to whatever expression precedes it rather than to an
    // identifier.
    both(
        &format!("{PICK}## Main\nIf pick(false) is none:\n    Show \"absent\".\n"),
        &["absent"],
    );
}

#[test]
fn the_question_can_be_asked_of_a_field() {
    // A record holding an optional is where this is asked most often — a decoded document, a row
    // with a nullable column.
    both(
        "## A Box has:\n\
        \x20   a tag, which is Option of Text.\n\
         \n\
         ## Main\n\
         Let b be a new Box with tag none.\n\
         If b's tag is none:\n\
        \x20   Show \"absent\".\n",
        &["absent"],
    );
}

#[test]
fn it_composes_with_and_the_way_a_guard_writes_it() {
    // The plan's own line is `If path of request starts with "/api" and currentUser(request) is
    // none:` — the predicate has to sit inside a conjunction, which is where a desugaring that
    // returned early from the wrong level would come apart.
    both(
        &format!(
            "{PICK}## Main\nLet p be \"/api/x\".\n\
             If startsWith(p, \"/api\") and pick(false) is none:\n\
            \x20   Show \"guarded\".\n"
        ),
        &["guarded"],
    );
}

#[test]
fn a_user_binding_named_none_still_wins() {
    // ★ THE DISAMBIGUATION, and the reason this is not simply a new keyword: `none` is an
    // ordinary word a program may bind (`Let none be ...`), and the `none` LITERAL already
    // yields to a live binding. The predicate follows the same rule, so a program that named a
    // variable `none` keeps the equality it always had rather than silently changing meaning.
    both(
        "## Main\n\
         Let none be 3.\n\
         Let x be 3.\n\
         If x is none:\n\
        \x20   Show \"equal\".\n",
        &["equal"],
    );
}

#[test]
fn an_absent_option_of_a_record_answers_it_too() {
    // The transparent encoding is what made `Option of <record>` hard to inspect in the first
    // place (see this file's header), so the predicate is asserted on that shape as well.
    both(
        "## A Response has:\n\
        \x20   a status, which is Int.\n\
         \n\
         ## To guard (deny: Bool) -> Option of Response:\n\
        \x20   If deny:\n\
        \x20       Return some(a new Response with status 401).\n\
        \x20   Return none.\n\
         \n\
         ## Main\n\
         If guard(false) is none:\n\
        \x20   Show \"passed\".\n\
         If guard(true) is not none:\n\
        \x20   Show \"answered\".\n",
        &["passed", "answered"],
    );
}
