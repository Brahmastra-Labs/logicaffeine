//! ★ LOCK — a cycle of derived state is refused, and the refusal names the cycle. ★
//!
//! `Let a always be b + 1.` beside `Let b always be a + 1.` has no fixed point: recomputing them
//! gives whichever answer the order happened to produce, and the turn semantics forbid a second
//! round. The dependency walk knows this — `DepError::DerivedCycle` carries **every field on the
//! cycle, in the order the walk met them**, and its own doc comment says *"better refused where it
//! is written than settled by whichever value happened to land last."*
//!
//! ★ IT IS NEVER REFUSED THERE. `component_deps` is called at exactly two sites and both discard
//! the error, each under the SAME comment — *"A component whose graph will not build (a derived
//! cycle) is refused elsewhere"*. There is no elsewhere. What the author actually gets is:
//!
//! ```text
//! Undefined variable: b
//! ```
//!
//! — which names a field they declared, calls it undefined, and says nothing about the circle that
//! is the real problem. A reader chasing that message goes looking for a typo in a name that is
//! spelled correctly.
//!
//! Same shape as `html_of_refusal_lock` and the `matchRoute` demand bug: a reason computed in full
//! at the one place that had it, then dropped, so the failure surfaces somewhere that cannot
//! explain it. The fix is never to write a better message — it is to report the one already built.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};
use logicaffeine_compile::semantics::dom;

fn component(state: &str) -> String {
    format!("## Component App\nState:\n{state}View:\n\x20   <p>{{a}}</p>\n\n## Main\nMount App.\n")
}

fn failure(subject: &str, src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("`{subject}` must be refused:\n{src}"))
}

#[test]
fn a_two_field_cycle_is_refused_and_names_both_fields() {
    let err = failure(
        "a two-field cycle",
        &component("\x20   Let a always be b + 1.\n\x20   Let b always be a + 1.\n"),
    );
    assert!(
        !err.contains("Undefined variable"),
        "the author declared both fields; calling one undefined sends them looking for a typo \
         that is not there: {err}"
    );
    assert!(
        err.contains('a') && err.contains('b'),
        "both fields on the cycle must be named: {err}"
    );
    let lower = err.to_lowercase();
    assert!(
        lower.contains("circle") || lower.contains("cycle") || lower.contains("each other"),
        "and the refusal must say what is actually wrong: {err}"
    );
}

#[test]
fn a_longer_cycle_is_refused_too() {
    // ★ NOT A TWO-FIELD SPECIAL CASE. The walk reports every field it met, so a three-hop circle
    // must name all three — that is the whole reason the error carries a list.
    let err = failure(
        "a three-field cycle",
        &component(
            "\x20   Let a always be c + 1.\n\x20   Let b always be a + 1.\n\
            \x20   Let c always be b + 1.\n",
        ),
    );
    assert!(!err.contains("Undefined variable"), "{err}");
    for f in ["a", "b", "c"] {
        assert!(
            err.contains(f),
            "field `{f}` is on the cycle and must be named: {err}"
        );
    }
}

#[test]
fn a_field_deriving_from_itself_is_refused() {
    let err = failure(
        "a self-cycle",
        &component("\x20   Let a always be a + 1.\n"),
    );
    assert!(!err.contains("Undefined variable"), "{err}");
    assert!(err.contains('a'), "{err}");
}

#[test]
fn an_ordinary_derived_chain_still_works() {
    // ★ THE FLOOR, and the thing a careless cycle check breaks first: derived state that depends
    // on other derived state is the normal case, not the error. `c` reads `b` reads `a`.
    dom::reset();
    let src = "## Component App\nState:\n\x20   Let a be 2.\n\
              \x20   Let b always be a * 2.\n\x20   Let c always be b + 1.\n\
               View:\n\x20   <p>{c}</p>\n\n## Main\nShow html of App.\n";
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "a derived chain must still build:\n{src}");
    assert_eq!(out.output.trim(), "<p>5</p>");
}

#[test]
fn a_component_with_no_derived_state_is_untouched() {
    dom::reset();
    let src = "## Component App\nState:\n\x20   Let a be 1.\nView:\n\x20   <p>{a}</p>\n\n\
               ## Main\nShow html of App.\n";
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(out.output.trim(), "<p>1</p>");
}

#[test]
fn the_deploy_tier_refuses_a_cycle_too() {
    // ★ AOT-Rust is the tier a server ships as and the only one that runs the static type checker,
    // so a refusal that lived only on the tiers the tests use would be the exact split
    // `server_surface_deploys_lock` exists about. A cycle is decided in the LOWERING, before any
    // tier runs, which is why the same sentence comes back here.
    let src = "## Component App\nState:\n\x20   Let a always be b + 1.\n\
              \x20   Let b always be a + 1.\nView:\n\x20   <p>{a}</p>\n\n\
               ## Main\nShow html of App.\n";
    match compile_to_rust(src) {
        Ok(rust) => {
            let refused: Vec<&str> = rust
                .lines()
                .filter(|l| l.contains("compile_error!"))
                .collect();
            assert!(
                !refused.is_empty(),
                "the deploy tier must refuse a derived cycle:\n{rust}"
            );
        }
        Err(e) => {
            let msg = format!("{e:?}");
            assert!(!msg.contains("Undefined variable"), "{msg}");
            assert!(msg.contains('a') && msg.contains('b'), "{msg}");
        }
    }
}
