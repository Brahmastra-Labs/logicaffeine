//! ★ LOCK — a host can call back into a page the TREE-WALKER has already run. ★
//!
//! The twin of `vm_session_lock` for the tree-walker. A browser instantiates a page's module, runs
//! `main` once, and from then on calls INTO it: an event handler, a fetch completion, a timer tick —
//! each is the host invoking a function by name or a closure the page handed it earlier, against the
//! globals `main` built. A failure unwinds to the host and leaves the page callable.
//!
//! The tree-walker becomes that page host's engine (`page_host::LoweredCalls`): the page's program
//! runs once through `Interpreter::run_page_main`, and then `call`/`call_value`/`has`/`reload`/`lines`
//! answer exactly what `vm::session::Session` answers — same globals model, same failure contract,
//! same refusals, same words. Every test here drives the seam from Rust, the way `page_host` does.

use logicaffeine_compile::ast::stmt::Stmt;
use logicaffeine_compile::interpreter::{Interpreter, RuntimeValue};
use logicaffeine_compile::page_host::LoweredCalls;
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use std::rc::Rc;

/// Parse `src` and hand `f` a fresh tree-walker over it together with the program's statements —
/// the page's main NOT yet run — and every `## Test` body, by name, as the script a host would run
/// beside the page.
fn with_page<R>(
    src: &str,
    f: impl for<'a> FnOnce(&mut Interpreter<'a>, &[Stmt<'a>], Vec<(String, &'a [Stmt<'a>])>) -> R,
) -> R {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, policies) =
            parsed.unwrap_or_else(|e| panic!("the program must parse:\n{src}\n=> {e}"));
        let page: Vec<Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::TestDef { .. }))
            .cloned()
            .collect();
        let tests = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::TestDef { name, body, .. } => {
                    Some((interner.resolve(*name).to_string(), *body))
                }
                _ => None,
            })
            .collect();
        let mut interp = Interpreter::new(interner)
            .with_type_registry(types)
            .with_policies(policies);
        f(&mut interp, &page, tests)
    })
}

fn int(n: i64) -> RuntimeValue {
    RuntimeValue::Int(n)
}

fn text(s: &str) -> RuntimeValue {
    RuntimeValue::Text(Rc::new(s.to_string()))
}

fn as_int(v: &RuntimeValue) -> i64 {
    match v {
        RuntimeValue::Int(n) => *n,
        other => panic!(
            "expected an Int, got {} ({})",
            other.type_name(),
            other.to_display_string()
        ),
    }
}

fn read(interp: &mut Interpreter<'_>, name: &str) -> i64 {
    as_int(
        &interp
            .call(name, vec![])
            .unwrap_or_else(|e| panic!("`{name}` must answer: {e}")),
    )
}

const COUNTER: &str = "\
## To bump:
    Set counter to counter + 1.

## To readCounter -> Int:
    Return counter.

## To addTo (a: Int) and (b: Int) -> Int:
    Return a + b + counter.

## Main
Let counter be 41.
";

#[test]
fn invoke_after_main_sees_main_globals() {
    with_page(COUNTER, |interp, page, _| {
        interp.run_page_main(page).expect("main runs");
        assert_eq!(read(interp, "readCounter"), 41, "main's binding is visible");
        interp.call("bump", vec![]).expect("bump runs");
        interp.call("bump", vec![]).expect("bump runs again");
        assert_eq!(
            read(interp, "readCounter"),
            43,
            "a write by one call is seen by the next — the globals persist between calls"
        );
        assert_eq!(
            as_int(&interp.call("addTo", vec![int(1), int(2)]).unwrap()),
            46,
            "arguments arrive in parameter order and the result comes back"
        );
    });
}

#[test]
fn a_host_call_answers_nothing_for_a_unit_function() {
    with_page(COUNTER, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        assert!(
            matches!(interp.call("bump", vec![]).unwrap(), RuntimeValue::Nothing),
            "a function with no result answers Nothing"
        );
    });
}

#[test]
fn the_interpreter_reports_which_functions_the_page_defines() {
    with_page(COUNTER, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        assert!(interp.has("bump") && interp.has("readCounter") && interp.has("addTo"));
        assert!(!interp.has("noSuchFunction"), "an absent name is absent");
        assert!(
            !interp.has("counter"),
            "a global is not a function the host can call"
        );
        assert!(
            !interp.has("show"),
            "a builtin is not a function the page defines"
        );
        let err = interp.call("noSuchFunction", vec![]).unwrap_err();
        assert!(
            err.contains("noSuchFunction"),
            "the refusal names the function: {err}"
        );
        let err = interp.call("counter", vec![]).unwrap_err();
        assert!(
            err.contains("counter"),
            "a global's name is refused as a function by name: {err}"
        );
        assert_eq!(
            read(interp, "readCounter"),
            41,
            "and the refusals touched nothing"
        );
    });
}

#[test]
fn invoking_before_main_is_refused() {
    // A browser cannot call an export of a module it has not instantiated, and main is what
    // registers every function and initializes the globals every function reads.
    with_page(COUNTER, |interp, _page, _| {
        assert!(
            !interp.has("readCounter"),
            "nothing is defined until the page's main has run"
        );
        let err = interp.call("readCounter", vec![]).unwrap_err();
        assert!(
            err.contains("main"),
            "the refusal says main has not run: {err}"
        );
        let err = interp.call_value(&text("x"), vec![]).unwrap_err();
        assert!(
            err.contains("main"),
            "a closure call is refused the same way: {err}"
        );
        let err = interp.reload().unwrap_err();
        assert!(
            err.contains("main"),
            "and so is a reload of a page that never loaded: {err}"
        );
    });
}

#[test]
fn main_runs_once() {
    with_page(COUNTER, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        interp.call("bump", vec![]).unwrap();
        let err = interp.run_page_main(page).unwrap_err();
        assert!(
            err.contains("main"),
            "a second main would re-initialize a live page: {err}"
        );
        assert_eq!(
            read(interp, "readCounter"),
            42,
            "and the refusal touched nothing"
        );
    });
}

#[test]
fn a_wrong_argument_count_is_refused_by_name() {
    with_page(COUNTER, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        let err = interp.call("addTo", vec![int(1)]).unwrap_err();
        assert!(
            err.contains("addTo") && err.contains('2') && err.contains('1'),
            "{err}"
        );
        assert_eq!(
            as_int(&interp.call("addTo", vec![int(1), int(2)]).unwrap()),
            44,
            "still callable"
        );
    });
}

const TRAPS: &str = "\
## To boom -> Int:
    Set counter to 99.
    Let xs be [1].
    Return item 9 of xs.

## To deep (n: Int) -> Int:
    Repeat for i from 1 to 3:
        If n is 0:
            Return item 9 of [1].
        Set counter to counter + deep(n - 1).
    Return 0.

## To sumTo (n: Int) -> Int:
    Let total be 0.
    Repeat for i from 1 to n:
        Set total to total + i.
    Return total.

## To countdown (n: Int) -> Int:
    If n is at most 0:
        Return 0.
    Return countdown(n - 1).

## To shadow -> Int:
    Let counter be 1000.
    Return item 9 of [counter].

## To readCounter -> Int:
    Return counter.

## Main
Let counter be 1.
";

#[test]
fn a_failed_invoke_leaves_the_interpreter_usable() {
    with_page(TRAPS, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        let err = interp.call("boom", vec![]).unwrap_err();
        assert!(
            err.contains('9'),
            "the failure's own message comes back to the host: {err}"
        );
        // ★ THE FAILURE LEAVES THE GLOBALS AS IT FOUND THEM — the write before the failure stays,
        // exactly as a wasm trap leaves linear memory. A turn's rollback is the LOWERING's job (it
        // writes state back only at the end); the engine's job is to stay callable.
        assert_eq!(read(interp, "readCounter"), 99, "globals are intact");
        // A failure three calls deep, inside open `Repeat` loops, unwinds every frame and scope.
        assert!(
            interp.call("deep", vec![int(3)]).is_err(),
            "the nested failure is reported"
        );
        assert_eq!(
            as_int(&interp.call("sumTo", vec![int(4)]).unwrap()),
            10,
            "a loop after an unwound loop runs from a clean scope"
        );
        assert_eq!(read(interp, "readCounter"), 99);
        // A failed call's own local never leaks into the next call's view of the globals: the
        // frame it defined `counter` in is gone.
        assert!(interp.call("shadow", vec![]).is_err());
        assert_eq!(
            read(interp, "readCounter"),
            99,
            "the failed frame's local shadows nothing"
        );
        // A self-tail-call after a failure runs in constant depth — the failed call's tail-call
        // bookkeeping did not survive it.
        assert_eq!(
            as_int(&interp.call("countdown", vec![int(10_000)]).unwrap()),
            0
        );
    });
}

#[test]
fn a_failed_main_leaves_the_interpreter_usable() {
    // A page whose main fails is still a page a browser can call into.
    let src =
        "## To answer -> Int:\n    Return 7.\n\n## Main\nLet xs be [1].\nShow item 5 of xs.\n";
    with_page(src, |interp, page, _| {
        assert!(
            interp.run_page_main(page).is_err(),
            "main's failure is reported"
        );
        assert_eq!(read(interp, "answer"), 7);
    });
}

#[test]
fn host_calls_do_not_grow_the_interpreter() {
    // ★ EVERY HOST CALL GIVES BACK WHAT IT TOOK. The call depth is bounded
    // (`semantics::MAX_CALL_DEPTH`); a host call that leaked even one level of it would refuse
    // every call after a few thousand failures — long before a page has answered a few hundred
    // thousand events. Both the clean return and the failure are exercised, because the failure
    // path is the one that forgets to clean up.
    let budget = logicaffeine_compile::semantics::MAX_CALL_DEPTH;
    with_page(TRAPS, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        for i in 0..budget * 20 {
            assert_eq!(
                as_int(&interp.call("sumTo", vec![int(3)]).unwrap()),
                6,
                "call {i}"
            );
        }
        for i in 0..budget * 4 {
            let err = interp.call("deep", vec![int(2)]).unwrap_err();
            assert!(
                !err.contains(logicaffeine_compile::semantics::CALL_DEPTH_ERR),
                "failure {i} hit the depth limit — an earlier failure leaked call depth: {err}"
            );
        }
        assert_eq!(as_int(&interp.call("sumTo", vec![int(5)]).unwrap()), 15);
    });
}

const HANDED_OVER: &str = "\
## To remember (t: Text):
    Set got to t.

## To answer -> Text:
    Return got.

## Main
Let got be \"nothing yet\".
Let onDone be (r: Int) -> remember(responseBody(r)).
Call fetchText with \"/api/x\" and onDone.
";

#[test]
fn a_closure_handed_to_a_builtin_is_callable_later() {
    dom::reset();
    with_page(HANDED_OVER, |interp, page, _| {
        interp
            .run_page_main(page)
            .expect("main registers the request");
        let sent = dom::fetch_requests();
        assert_eq!(sent.len(), 1, "main sent exactly one request: {sent:?}");
        let req = sent[0].req;
        let closure = dom::begin_fetch_resolve(req, 200, &[], "pong")
            .expect("the request holds the closure main handed it");
        assert!(
            matches!(closure, RuntimeValue::Function(_)),
            "and it is a closure"
        );
        let out = interp.call_value(&closure, vec![int(req)]);
        dom::end_fetch_resolve();
        out.expect("the completion runs inside the interpreter");
        match interp.call("answer", vec![]).unwrap() {
            RuntimeValue::Text(t) => {
                assert_eq!(t.as_str(), "pong", "the completion wrote the global")
            }
            other => panic!("expected Text, got {}", other.type_name()),
        }
    });
}

#[test]
fn calling_a_value_that_is_not_a_closure_is_refused() {
    with_page(COUNTER, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        let err = interp.call_value(&text("hi"), vec![]).unwrap_err();
        assert!(
            err.contains("Text"),
            "the refusal names what was called: {err}"
        );
        assert_eq!(read(interp, "readCounter"), 41, "still callable");
    });
}

#[test]
fn a_closure_with_the_wrong_argument_count_is_refused() {
    dom::reset();
    with_page(HANDED_OVER, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        let req = dom::fetch_requests()[0].req;
        let closure = dom::begin_fetch_resolve(req, 200, &[], "x").unwrap();
        dom::end_fetch_resolve();
        let err = interp.call_value(&closure, vec![]).unwrap_err();
        assert!(err.contains('1') && err.contains('0'), "{err}");
        interp
            .call_value(&closure, vec![int(req)])
            .expect("and the right count still works");
    });
}

#[test]
fn output_of_every_call_accumulates_in_order() {
    let src = "## To say (t: Text):\n    Show t.\n\n## Main\nShow \"main\".\n";
    with_page(src, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        interp.call("say", vec![text("one")]).unwrap();
        interp.call("say", vec![text("two")]).unwrap();
        assert_eq!(interp.lines(), ["main", "one", "two"]);
    });
}

const RELOADS: &str = "\
## To bump:
    Set counter to counter + 1.

## To readCounter -> Int:
    Return counter.

## Main
Let counter be 41.
Let seen be [1, 2].

## Test: a script beside the page
Let mine be 5.
Let alsoMine be \"kept\".
Expect mine is 5.
";

/// The value `name` holds among the interpreter's globals, as `Show` renders it.
fn global(interp: &Interpreter<'_>, name: &str) -> Option<String> {
    interp
        .global_bindings()
        .into_iter()
        .find(|(n, _, _)| n == name)
        .map(|(_, _, v)| v)
}

#[test]
fn reload_resets_page_globals_and_keeps_test_bindings() {
    with_page(RELOADS, |interp, page, tests| {
        interp.run_page_main(page).unwrap();
        let script = tests.first().map(|(_, body)| *body).expect("the script");
        interp.enable_test_mode(50_000_000);
        interp
            .run_sync(script)
            .expect("the script runs beside the page");
        interp.call("bump", vec![]).unwrap();
        interp.call("bump", vec![]).unwrap();
        assert_eq!(read(interp, "readCounter"), 43);
        interp.reload().expect("the page reloads");
        assert_eq!(
            read(interp, "readCounter"),
            41,
            "a reload is the page's main run again: every global it builds starts over"
        );
        assert_eq!(global(interp, "seen").as_deref(), Some("[1, 2]"));
        assert_eq!(
            global(interp, "mine").as_deref(),
            Some("5"),
            "the script's own bindings are not the page's, and a reload leaves them alone"
        );
        assert_eq!(global(interp, "alsoMine").as_deref(), Some("kept"));
        interp.call("bump", vec![]).unwrap();
        assert_eq!(
            read(interp, "readCounter"),
            42,
            "the reloaded page answers calls as before"
        );
    });
}

#[test]
fn a_reload_runs_the_pages_main_again_and_its_output_accumulates() {
    let src = "## To say (t: Text):\n    Show t.\n\n## Main\nShow \"loaded\".\n";
    with_page(src, |interp, page, _| {
        interp.run_page_main(page).unwrap();
        interp.call("say", vec![text("one")]).unwrap();
        interp.reload().unwrap();
        assert_eq!(interp.lines(), ["loaded", "one", "loaded"]);
    });
}

const SIGN_IN: &str = "\
## Store Session
State:
    Let name be \"guest\".
Actions:
    To login (who: Text):
        Set name to who.

## Component App
Use Session.
Actions:
    To signIn:
        Call Session's login with \"ada\".
View:
    <p id=\"who\">{Session's name}</p>
    <button id=\"go\" on-click={signIn}>in</button>

## Test: the script reads the store the page wrote
Mount App.
Click \"#go\".
Show Session's name.
";

/// ★ THE PAGE'S STORE IS THE RECORD ITS LOWERED PRELUDE BINDS. A lowered page keeps a store as an
/// ordinary global record named after the store, and its actions write that record; a script beside
/// the page that reads `Session's name` reads the very same record, never a copy of its own.
#[test]
fn a_script_reads_the_store_record_the_lowered_page_wrote() {
    dom::reset();
    with_page(SIGN_IN, |interp, page, tests| {
        interp
            .host_lowered_page(page, None)
            .expect("the page starts");
        let script = tests.first().map(|(_, body)| *body).expect("the script");
        interp.enable_test_mode(50_000_000);
        interp
            .run_statements(script)
            .expect("the script runs beside the page");
        assert!(
            dom::render().contains("<p id=\"who\">ada</p>"),
            "the lowered action wrote the store and repainted: {}",
            dom::render()
        );
        assert_eq!(
            interp.lines().last().map(String::as_str),
            Some("ada"),
            "{:?}",
            interp.lines()
        );
        assert_eq!(interp.hosted_page_failure(), None);
    });
}

const READS_STORAGE: &str = "\
## To readSaved -> Text:
    Return saved.

## Main
Let saved be storageGetOr(\"local\", \"greeting\", \"nothing\").
";

/// ★ A PAGE'S MAIN RUNS IN THE WORLD ITS HOST HOLDS. A browser loading a module does not clear
/// the visitor's storage: what earlier visits left is there when the page's main reads it. The
/// storage profile is the HOST's, as the document is, so the engine running the page's main starts
/// itself and nothing else — exactly as `vm::session::Session::run_main` does.
#[test]
fn a_pages_main_reads_the_storage_its_host_holds() {
    use logicaffeine_compile::semantics::storage;
    with_page(READS_STORAGE, |interp, page, _| {
        storage::reset();
        storage::set(storage::LOCAL, "greeting", "hello again").unwrap();
        interp.run_page_main(page).expect("main runs");
        match interp.call("readSaved", vec![]).expect("readSaved answers") {
            RuntimeValue::Text(t) => assert_eq!(
                t.as_str(),
                "hello again",
                "main read what the host's storage held"
            ),
            other => panic!("a Text, got {}", other.type_name()),
        }
        assert_eq!(
            storage::get(storage::LOCAL, "greeting").unwrap().as_deref(),
            Some("hello again"),
            "and running the page's main left the host's storage as it was"
        );
    });
}
