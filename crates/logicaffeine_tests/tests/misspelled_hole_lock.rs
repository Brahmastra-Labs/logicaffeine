//! ★ LOCK — a misspelt hole is refused at build time, by name. ★
//!
//! `{nme}` beside a `name` field passes the template parser, the LGX checker, the type checker
//! and the whole build. It fails when a visitor loads the page. For a template language that is
//! the most common authoring mistake there is, and it is the one a compiler is best placed to
//! catch: the names a hole may use are exactly the component's own state, props, actions and
//! template bindings.
//!
//! ★ THE RULE IS A NEAR MISS, NOT AN UNKNOWN, and that distinction is the whole design. A hole
//! may legitimately name a module-level function or a stdlib value the checker cannot see, so
//! "not in the component's scope" is not evidence of a mistake. One edit from a name that IS in
//! scope is. False negatives are accepted deliberately — a typo two edits out goes unreported —
//! because a false positive teaches authors to ignore the checker, which costs more than the
//! feature is worth.

use logicaffeine_compile::compile::tw_outcome;

fn page(state: &str, hole: &str) -> String {
    format!(
        "## Component App\nState:\n\x20   {state}\n\
         Actions:\n\x20   To go:\n\x20       Set n to n + 1.\n\
         View:\n\x20   <p id=\"p\">{{{hole}}}</p>\n\n\
         ## Main\nMount App.\nShow the text of \"#p\".\n"
    )
}

#[test]
fn a_hole_one_edit_from_a_state_field_is_refused_and_names_the_fix() {
    let src = page("Let name be \"Ada\".\n\x20   Let n be 0.", "nme");
    let out = tw_outcome(&src);
    let e = out.error.unwrap_or_else(|| {
        panic!(
            "`{{nme}}` beside a `name` field must be refused, not deferred to a visitor's \
                browser. It printed {:?}\n{src}",
            out.output
        )
    });
    assert!(
        e.contains("nme"),
        "the refusal must quote what was written — got {e:?}"
    );
    assert!(e.contains("name"), "and name the fix — got {e:?}");
}

#[test]
fn the_correctly_spelled_hole_still_renders() {
    // ★ THE CONTROL. A check that refused the right spelling too would be caught here, and
    // nothing else in this file would notice.
    let src = page("Let name be \"Ada\".\n\x20   Let n be 0.", "name");
    let out = tw_outcome(&src);
    assert_eq!(
        out.error, None,
        "a correctly spelled hole must render:\n{src}"
    );
    assert!(out.output.contains("Ada"), "got {:?}", out.output);
}

#[test]
fn a_hole_naming_something_far_from_every_field_is_left_alone() {
    // ★ THE FALSE-POSITIVE GUARD, and the reason this is a near-miss rule. A hole may name a
    // module-level function or a stdlib value the checker cannot see. `shout` is two or more
    // edits from every name in scope, so the checker must stay silent and let the ordinary
    // name resolution decide — whatever it decides.
    let src = format!(
        "## To shout (t: Text) -> Text:\n\x20   Return t + \"!\".\n\n\
         ## Component App\nState:\n\x20   Let name be \"Ada\".\n\
         Actions:\n\x20   To go:\n\x20       Set name to \"x\".\n\
         View:\n\x20   <p id=\"p\">{{shout(name)}}</p>\n\n\
         ## Main\nMount App.\nShow the text of \"#p\".\n"
    );
    let out = tw_outcome(&src);
    assert_eq!(
        out.error, None,
        "a hole calling a module function must not be flagged:\n{src}"
    );
    assert!(out.output.contains("Ada!"), "got {:?}", out.output);
}

#[test]
fn a_loop_variable_is_in_scope_for_the_holes_inside_the_loop() {
    // A `{for each}` binding is part of the component's vocabulary. Flagging `{t}` because it is
    // not a State: field would make the checker unusable on the first list anybody writes.
    let src = "## Component App\nState:\n\x20   Let items be [\"a\", \"b\"].\n\
               Actions:\n\x20   To go:\n\x20       Set items to [\"c\"].\n\
               View:\n\x20   <ul id=\"u\">{for each t in items:}<li>{t}</li>{end}</ul>\n\n\
               ## Main\nMount App.\nShow the text of \"#u\".\n";
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "a loop variable must be in scope:\n{src}");
    assert!(
        out.output.contains('a') && out.output.contains('b'),
        "got {:?}",
        out.output
    );
}
