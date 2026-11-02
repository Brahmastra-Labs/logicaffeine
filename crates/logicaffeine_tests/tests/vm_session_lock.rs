//! ★ LOCK — a host can call back into a program the bytecode VM has already run. ★
//!
//! A browser instantiates a page's module, runs `main` once, and from then on calls INTO it: an
//! event handler, a fetch completion, a timer tick — each is the host invoking an export or a
//! closure the module handed it earlier. A trap unwinds to the host and leaves the instance
//! callable, with its memory exactly as the trap left it.
//!
//! `vm::session::Session` is that shape for the in-process VM: `run_main` once, then `invoke(name, args)`
//! and `call_value(closure, args)` against the globals `main` built. Each test here drives the
//! seam from Rust, the way `page_host` does, and asserts what a browser host can rely on.

use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::session::Session;
use logicaffeine_compile::vm::{Compiler, Vm};
use std::rc::Rc;

/// Compile `src`, start a session over it (main NOT yet run), and hand it to `f`.
fn with_session<R>(src: &str, f: impl FnOnce(&mut Session<'_>) -> R) -> R {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) =
            parsed.unwrap_or_else(|e| panic!("the program must parse:\n{src}\n=> {e}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("the program must compile:\n{src}\n=> {e}"));
        let mut session = Session::new(Vm::new(&program), interner);
        f(&mut session)
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
    with_session(COUNTER, |s| {
        s.run_main().expect("main runs");
        assert_eq!(
            as_int(&s.invoke("readCounter", vec![]).unwrap()),
            41,
            "main's binding is visible"
        );
        s.invoke("bump", vec![]).expect("bump runs");
        s.invoke("bump", vec![]).expect("bump runs again");
        assert_eq!(
            as_int(&s.invoke("readCounter", vec![]).unwrap()),
            43,
            "a write by one invocation is seen by the next — the globals persist between calls"
        );
        assert_eq!(
            as_int(&s.invoke("addTo", vec![int(1), int(2)]).unwrap()),
            46,
            "arguments arrive in parameter order and the result comes back"
        );
    });
}

#[test]
fn a_host_call_answers_nothing_for_a_unit_function() {
    with_session(COUNTER, |s| {
        s.run_main().unwrap();
        assert!(
            matches!(s.invoke("bump", vec![]).unwrap(), RuntimeValue::Nothing),
            "a function with no result answers Nothing"
        );
    });
}

#[test]
fn a_session_reports_which_functions_it_has() {
    with_session(COUNTER, |s| {
        assert!(s.has("bump") && s.has("readCounter") && s.has("addTo"));
        assert!(!s.has("noSuchFunction"), "an absent name is absent");
        let err = s.invoke("noSuchFunction", vec![]).unwrap_err();
        assert!(
            err.contains("noSuchFunction"),
            "the refusal names the function: {err}"
        );
    });
}

#[test]
fn invoking_before_main_is_refused() {
    // A browser cannot call an export of a module it has not instantiated, and main is what
    // initializes the globals every function reads. Calling first would read `counter` unbound.
    with_session(COUNTER, |s| {
        let err = s.invoke("readCounter", vec![]).unwrap_err();
        assert!(
            err.contains("main"),
            "the refusal says main has not run: {err}"
        );
    });
}

#[test]
fn main_runs_once() {
    with_session(COUNTER, |s| {
        s.run_main().unwrap();
        s.invoke("bump", vec![]).unwrap();
        assert!(
            s.run_main().is_err(),
            "a second main would re-initialize a live page"
        );
        assert_eq!(
            as_int(&s.invoke("readCounter", vec![]).unwrap()),
            42,
            "and the refusal touched nothing"
        );
    });
}

#[test]
fn a_wrong_argument_count_is_refused_by_name() {
    with_session(COUNTER, |s| {
        s.run_main().unwrap();
        let err = s.invoke("addTo", vec![int(1)]).unwrap_err();
        assert!(
            err.contains("addTo") && err.contains('2') && err.contains('1'),
            "{err}"
        );
        assert_eq!(
            as_int(&s.invoke("addTo", vec![int(1), int(2)]).unwrap()),
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

## To readCounter -> Int:
    Return counter.

## Main
Let counter be 1.
";

#[test]
fn a_failed_invoke_leaves_the_session_usable() {
    with_session(TRAPS, |s| {
        s.run_main().unwrap();
        let err = s.invoke("boom", vec![]).unwrap_err();
        assert!(
            err.contains('9'),
            "the trap's own message comes back to the host: {err}"
        );
        // ★ THE TRAP LEAVES MEMORY AS IT FOUND IT — the write before the trap stays, exactly as a
        // wasm trap leaves linear memory. A turn's rollback is the LOWERING's job (it writes state
        // back only at the end); the machine's job is to stay callable.
        assert_eq!(
            as_int(&s.invoke("readCounter", vec![]).unwrap()),
            99,
            "globals are intact"
        );
        // A trap three calls deep, inside open `Repeat` loops, unwinds every frame and iterator.
        assert!(
            s.invoke("deep", vec![int(3)]).is_err(),
            "the nested trap is reported"
        );
        assert_eq!(
            as_int(&s.invoke("sumTo", vec![int(4)]).unwrap()),
            10,
            "a loop after an unwound loop runs from a clean iterator stack"
        );
        assert_eq!(as_int(&s.invoke("readCounter", vec![]).unwrap()), 99);
    });
}

#[test]
fn a_failed_main_leaves_the_session_usable() {
    // A page whose main traps is still an instance a browser can call into.
    let src =
        "## To answer -> Int:\n    Return 7.\n\n## Main\nLet xs be [1].\nShow item 5 of xs.\n";
    with_session(src, |s| {
        assert!(s.run_main().is_err(), "main's trap is reported");
        assert_eq!(as_int(&s.invoke("answer", vec![]).unwrap()), 7);
    });
}

#[test]
fn host_calls_do_not_grow_the_machine() {
    // ★ EVERY HOST CALL GIVES BACK WHAT IT TOOK. The register file is bounded (1 << 20 slots); a
    // host frame that leaked even its argument window would exhaust it long before a page that
    // answers a few hundred thousand events. Both the clean return and the trap are exercised,
    // because the trap path is the one that forgets to clean up.
    with_session(TRAPS, |s| {
        s.run_main().unwrap();
        for i in 0..200_000 {
            assert_eq!(
                as_int(&s.invoke("sumTo", vec![int(3)]).unwrap()),
                6,
                "call {i}"
            );
        }
        for i in 0..100_000 {
            assert!(s.invoke("deep", vec![int(2)]).is_err(), "trap {i}");
        }
        assert_eq!(as_int(&s.invoke("sumTo", vec![int(5)]).unwrap()), 15);
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
    with_session(HANDED_OVER, |s| {
        s.run_main().expect("main registers the request");
        let sent = dom::fetch_requests();
        assert_eq!(sent.len(), 1, "main sent exactly one request: {sent:?}");
        let req = sent[0].req;
        let closure = dom::begin_fetch_resolve(req, 200, &[], "pong")
            .expect("the request holds the closure main handed it");
        assert!(
            matches!(closure, RuntimeValue::Function(_)),
            "and it is a closure"
        );
        let out = s.call_value(&closure, vec![int(req)]);
        dom::end_fetch_resolve();
        out.expect("the completion runs inside the session");
        match s.invoke("answer", vec![]).unwrap() {
            RuntimeValue::Text(t) => {
                assert_eq!(t.as_str(), "pong", "the completion wrote the global")
            }
            other => panic!("expected Text, got {}", other.type_name()),
        }
    });
}

#[test]
fn calling_a_value_that_is_not_a_closure_is_refused() {
    with_session(COUNTER, |s| {
        s.run_main().unwrap();
        let err = s.call_value(&text("hi"), vec![]).unwrap_err();
        assert!(
            err.contains("Text"),
            "the refusal names what was called: {err}"
        );
        assert_eq!(
            as_int(&s.invoke("readCounter", vec![]).unwrap()),
            41,
            "still callable"
        );
    });
}

#[test]
fn a_closure_with_the_wrong_argument_count_is_refused() {
    dom::reset();
    with_session(HANDED_OVER, |s| {
        s.run_main().unwrap();
        let req = dom::fetch_requests()[0].req;
        let closure = dom::begin_fetch_resolve(req, 200, &[], "x").unwrap();
        dom::end_fetch_resolve();
        let err = s.call_value(&closure, vec![]).unwrap_err();
        assert!(err.contains('1') && err.contains('0'), "{err}");
        s.call_value(&closure, vec![int(req)])
            .expect("and the right count still works");
    });
}

#[test]
fn output_of_every_call_accumulates_in_order() {
    let src = "## To say (t: Text):\n    Show t.\n\n## Main\nShow \"main\".\n";
    with_session(src, |s| {
        s.run_main().unwrap();
        s.invoke("say", vec![text("one")]).unwrap();
        s.invoke("say", vec![text("two")]).unwrap();
        assert_eq!(s.lines(), ["main", "one", "two"]);
    });
}
