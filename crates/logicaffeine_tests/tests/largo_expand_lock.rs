//! ★ LOCK — `largo expand` shows you what your page BECAME. ★
//!
//! An LGX page is not run as written. A `## Component` is lowered into ordinary LOGOS: a render
//! function of `createElement`/`setAttribute`/`appendChild` calls, an action function per action, a
//! mount entry, a server-side HTML builder, a dispatcher the driving verbs route through — and a
//! `## Route` becomes a function plus a synthesized `route(method, path)` walk. That lowering is
//! the thing that actually runs, and until now there was no way to LOOK at it.
//!
//! ★ IT IS A DEBUGGING TOOL, SO IT MUST SHOW THE REAL LOWERING. Printing a summary someone wrote
//! by hand would be a second description of the compiler, free to drift from it and to describe a
//! program the compiler no longer emits — the failure this campaign keeps finding. So it walks the
//! statements the parser actually produced.
//!
//! ★ AND IT MUST NOT LIE BY OMISSION. A construct the compiler refuses is exactly what somebody
//! reaches for this tool to understand, so a page that does not lower must say so rather than
//! print an empty list that reads like "nothing to see".

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};

/// Run `largo expand` over a source file written into a temp dir.
fn expand(source: &str) -> (bool, String, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("page.lg");
    std::fs::write(&file, source).expect("write source");
    let out = largo_in(dir.path(), &["expand", file.to_str().unwrap()]);
    (out.status.success(), stdout(&out), stderr(&out))
}

const PAGE: &str = "\
## Component Counter
State:
    Let clicks be 0.
Actions:
    To bump:
        Set clicks to clicks + 1.
View:
    <p id=\"n\">{clicks}</p>
    <button id=\"b\" on-click={bump}>+</button>

## Main
Mount Counter.
";

#[test]
fn it_names_the_functions_a_component_lowers_to() {
    // The whole point: a component you wrote as markup and sentences becomes ORDINARY FUNCTIONS,
    // and this is where you see which.
    let (ok, out, err) = expand(PAGE);
    assert!(ok, "expand succeeds on a page that lowers:\n{err}");
    for want in ["__render_Counter", "__act_Counter_bump", "__mount_Counter"] {
        assert!(out.contains(want), "expand names `{want}`:\n{out}");
    }
}

#[test]
fn it_shows_the_tape_calls_the_view_became() {
    // A view is not markup at run time — it is the tape its render writes (`vdOpen`, `vdText`,
    // `vdClose`), which the commit turns into what the browser is told to do. Somebody reading
    // this wants to see the calls, because that is what the page actually runs.
    let (ok, out, err) = expand(PAGE);
    assert!(ok, "{err}");
    for want in ["vdOpen", "vdClose"] {
        assert!(
            out.contains(want),
            "the lowered view shows its `{want}` calls:\n{out}"
        );
    }
}

#[test]
fn it_shows_a_routes_lowering_too() {
    // The server half lowers as well: a `## Route` is a function plus the synthesized dispatcher
    // every `route(…)` call goes through.
    let (ok, out, err) = expand(
        "## Route GET /ping -> Text:\n    Return \"pong\".\n\n## Main\nShow route(\"GET\", \"/ping\").\n",
    );
    assert!(ok, "expand succeeds on a routed program:\n{err}");
    assert!(out.contains("route"), "the dispatcher appears:\n{out}");
}

#[test]
fn a_plain_program_expands_to_itself_and_says_so() {
    // ★ THE EDGE THAT MAKES THE OUTPUT HONEST. A program with no LGX in it has no lowering, and
    // printing an empty list would read like "the compiler ate your program". It has to say that
    // there was nothing to lower.
    let (ok, out, err) = expand("## Main\nShow 1 + 1.\n");
    assert!(ok, "a plain program expands:\n{err}");
    let lower = out.to_lowercase();
    assert!(
        lower.contains("no lgx") || lower.contains("nothing") || lower.contains("0 "),
        "it says there was nothing to lower rather than printing an empty list:\n{out}"
    );
}

#[test]
fn a_page_that_does_not_parse_fails_loudly_rather_than_printing_nothing() {
    // Somebody runs this BECAUSE something is wrong. A broken page that exited 0 with an empty
    // list would send them looking at the lowering when the problem is a syntax error.
    let (ok, _out, err) =
        expand("## Component App\nView:\n    <p>unclosed\n\n## Main\nMount App.\n");
    assert!(
        !ok,
        "a page that does not parse is an error, not an empty expansion"
    );
    assert!(!err.trim().is_empty(), "and it says what went wrong");
}

#[test]
fn a_missing_file_is_a_clear_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = largo_in(dir.path(), &["expand", "no-such-file.lg"]);
    assert!(!out.status.success(), "a missing file is an error");
    let err = stderr(&out).to_lowercase();
    assert!(
        err.contains("no-such-file") || err.contains("not found") || err.contains("no such"),
        "the error names the file: {err}"
    );
}

#[test]
fn a_repeated_stateful_component_expands_once_and_is_placed_twice() {
    // ★ MIGRATED, AND STRICTLY STRONGER. This used to assert that a stateful component placed twice
    // was REPORTED AS REFUSED — someone reaching for `expand` to find out why their page did not
    // render had to be told, because an output that simply lacked the function reads as a compiler
    // bug. That was the right test for a tier that could not render it.
    //
    // It can now. State is a map from instance key to value and a placement passes the key of the
    // instance it draws, so the two counters keep two records. The test that pinned the gap now
    // pins what `expand` is actually for: showing that ONE render function is emitted for the
    // component and that the page CALLS it twice — which is the whole shape of the lowering, and
    // the thing somebody runs this tool to see.
    let src = "\
## Component Counter
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p>{n}</p>

## Component App
View:
    <div><Counter/><Counter/></div>

## Main
Mount App.
";
    let (ok, out, err) = expand(src);
    assert!(ok, "a repeated stateful component expands:\n{out}\n{err}");
    // Its definition, plus the call each placement makes. Asserted as "more than once" rather
    // than an exact count: the point is that ONE function serves BOTH placements, and pinning the
    // precise number would pin this tool's formatting rather than the lowering it reports.
    let mentions = out.matches("__render_Counter").count();
    assert!(
        mentions >= 2,
        "one render function, called by each placement — got {mentions} mentions:\n{out}"
    );
    assert!(
        !out.to_lowercase().contains("refus"),
        "nothing is refused here any more:\n{out}"
    );
}

#[test]
fn the_expansion_is_deterministic() {
    // It is a debugging tool people diff between runs. Output that reordered itself would make
    // every diff noise, and the underlying registries really are hash maps.
    let (_, a, _) = expand(PAGE);
    let (_, b, _) = expand(PAGE);
    assert_eq!(a, b, "two expansions of one source agree");
}
