//! ★ LOCK — a `State:` field's declared type means what it means in a `Let`. ★
//!
//! `State:` is a block of `Let` sentences, and an author who writes `Let rate: Real be 3.` there
//! has said the same thing they would say anywhere else. The task board recorded that they had NOT:
//! plain LOGOS refused the mismatch while a `State:` block silently accepted it and then IGNORED
//! the annotation, so the field behaved as an Int and `rate / 2` rendered `1` instead of `1.5`.
//! "A declared type that is quietly disregarded is worse than no annotation, because the author has
//! been told their intent was understood."
//!
//! ★ IT IS FIXED, AND THIS FILE IS THE PROOF RATHER THAN THE FIX — measured, not assumed, which is
//! the honest description. Both halves now behave, and the interesting part is that they behave the
//! SAME WAY as each other in both directions:
//!
//!   * `Real be 3` is a WIDENING, not a mismatch — an Int is a Real, so it is honoured and the
//!     field really is a Real. That is why `rate / 2` is `1.5`: the value the author declared.
//!   * `Bool be 3` is a genuine mismatch and is REFUSED, in a `State:` block exactly as in a `Let`,
//!     with the same Socratic message. One rule, one wording, two places.
//!
//! The pair is the point. A lock on the refusal alone would pass against a `State:` block that
//! refused EVERYTHING, and a lock on the widening alone would pass against one that checked
//! nothing at all.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};

/// The answer both tiers give, held equal — a divergence here would mean the annotation is read by
/// one tier and not the other, which is the same silence in a narrower place.
fn both_tiers(src: &str) -> String {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "tree-walker vs VM:\n{src}"
    );
    tw.output.trim_end().to_string()
}

/// The refusal both tiers give, held equal.
fn refusal(src: &str) -> String {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    let a = tw
        .error
        .unwrap_or_else(|| panic!("must be refused:\n{src}"));
    let b = vm
        .error
        .unwrap_or_else(|| panic!("the VM must refuse it too:\n{src}"));
    assert_eq!(a, b, "both tiers must refuse it in the SAME WORDS:\n{src}");
    a
}

#[test]
fn a_real_state_field_initialized_with_an_int_is_a_real() {
    // ★ THE HEADLINE. `1.5` is the annotation being honoured; `1` would be it being ignored, which
    // is precisely the silent wrong answer this was recorded for.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let rate: Real be 3.\n\
       \n\
       View:\n\
      \x20   <p>{rate / 2}</p>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    assert_eq!(both_tiers(src), "<p>1.5</p>");
}

#[test]
fn the_same_annotation_in_a_plain_let_answers_the_same() {
    // The reference the `State:` block is being held to. If this ever stops widening, the test
    // above must move with it rather than the two drifting apart.
    assert_eq!(
        both_tiers("## Main\nLet rate: Real be 3.\nShow rate / 2.\n"),
        "1.5"
    );
}

#[test]
fn a_mismatched_state_field_is_refused() {
    // A Bool is not an Int in any direction, so this is a real mistake and must be caught where it
    // is written — not silently accepted and then disregarded.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let flag: Bool be 3.\n\
       \n\
       View:\n\
      \x20   <p>{flag}</p>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    let msg = refusal(src);
    assert!(
        msg.contains("Bool"),
        "the message must name the declared type: {msg}"
    );
}

#[test]
fn the_state_block_and_the_let_refuse_it_in_the_same_words() {
    // ★ ONE RULE, ONE MESSAGE. The `State:` block runs the check the `Let` path already runs and
    // reuses its wording — so an author who has met this error once recognises it, and there is no
    // second message to drift.
    let in_state = refusal(
        "## Component App\n\nState:\n    Let flag: Bool be 3.\n\nView:\n    <p>x</p>\n\n## Main\nShow html of App.\n",
    );
    let in_let = refusal("## Main\nLet flag: Bool be 3.\nShow flag.\n");
    assert_eq!(
        in_state, in_let,
        "a State: field and a Let must refuse identically"
    );
}

#[test]
fn a_correct_annotation_is_left_alone() {
    // The ordinary case: an annotation that agrees with its value changes nothing. A check that
    // fired on agreement would make every typed field an error.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let count: Int be 3.\n\
      \x20   Let name: Text be \"hi\".\n\
      \x20   Let done: Bool be false.\n\
       \n\
       View:\n\
      \x20   <p>{count}|{name}|{done}</p>\n\
       \n\
       ## Main\n\
       Show html of App.\n";
    assert_eq!(both_tiers(src), "<p>3|hi|false</p>");
}

#[test]
fn a_derived_field_annotation_behaves_exactly_like_a_lets() {
    // ★ THE PROPERTY, not a number I guessed. `always be` is a `Let` with a derivation behind it,
    // so whatever a top-level `Let` does with `: Real be n / 2`, a derived field must do too.
    //
    // What both do is compute FIRST and store SECOND: `n / 2` over two Ints is integer division,
    // and an annotation on the slot does not reach back and change the operator. That is why this
    // answers `1` and not `1.5` — and why the literal case above answers `1.5`, where the
    // annotation widens the VALUE before any arithmetic touches it. My first version of this test
    // asserted `1.5` from the shape of the sentence rather than from what the language does; the
    // claim worth locking is that the two positions AGREE, which is what a reader relies on.
    let derived = both_tiers(
        "## Component App\n\nState:\n    Let n be 3.\n    Let half: Real always be n / 2.\n\nView:\n    <p>{half}</p>\n\n## Main\nShow html of App.\n",
    );
    let plain_state = both_tiers(
        "## Component App\n\nState:\n    Let n be 3.\n    Let half: Real be n / 2.\n\nView:\n    <p>{half}</p>\n\n## Main\nShow html of App.\n",
    );
    let top_level = both_tiers("## Main\nLet n be 3.\nLet half: Real be n / 2.\nShow half.\n");
    assert_eq!(
        derived, plain_state,
        "a derived field and a plain one must agree"
    );
    assert_eq!(
        derived,
        format!("<p>{top_level}</p>"),
        "and both must agree with a top-level `Let`"
    );
}

#[test]
fn a_typed_state_field_reaches_the_browser_tier() {
    // What a visitor runs. The annotation has to survive the lowering, not just the reference tier.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let rate: Real be 3.\n\
       \n\
       View:\n\
      \x20   <p>{rate / 2}</p>\n\
       \n\
       ## Main\n\
       Mount App.\n";
    let module =
        compile_to_wasm(src).unwrap_or_else(|e| panic!("must reach the browser tier: {e:?}"));
    let engine = wasmi::Engine::default();
    wasmi::Module::new(&engine, &module[..])
        .unwrap_or_else(|e| panic!("the module must VALIDATE, or no browser loads it: {e}"));
}
