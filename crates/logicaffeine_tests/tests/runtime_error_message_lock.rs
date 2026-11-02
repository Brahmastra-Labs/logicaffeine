// The emitted module is RUN under the `wasmi` interpreter, gated — like the rest of the
// WASM run-tests — behind the `wasm-jit` feature. Registered in `scripts/test/wasm-jit-filter.txt`.
#![cfg(feature = "wasm-jit")]
//! ★ LOCK — a runtime failure reaches the browser WITH ITS MESSAGE. ★
//!
//! The compiler goes to real trouble to author these messages — "`Mount App at "#root"`: no
//! element matches that selector. Create the mount point before mounting, or mount without `at`
//! to render under the page body." — interns each one as a `Constant::Text`, and hands the
//! bytecode an `Op::FailWith { msg: ConstIdx }` naming it. The tree-walker prints it. The VM
//! returns it verbatim (`machine.rs`: `Constant::Text(s) => s.clone()`).
//!
//! And the WASM backend THREW IT AWAY — `module.rs`, on the arm for the same op:
//!
//! ```text
//! // The message constant is intentionally dropped.
//! Op::FailWith { .. } => { code.push(0x00); /* unreachable → trap */ }
//! ```
//!
//! So on the one tier a visitor actually runs, every failed `Check`, every out-of-range index,
//! every mount selector that matches nothing surfaced as:
//!
//! ```text
//! Uncaught (in promise) RuntimeError: unreachable
//! ```
//!
//! No message, no line, no source. For a language whose whole pitch is that its errors teach
//! you something, the browser tier said nothing at all.
//!
//! ★ THE TRAP CONTRACT IS UNCHANGED, AND THAT IS THE POINT. `wasm_aot_lock`'s
//! `wasm_traps_where_treewalker_errors` proves tw-errors ⟺ wasm-traps, and it must stay green:
//! a standalone module signals failure by trapping, and nothing here makes it stop. The host is
//! simply TOLD first. `logos_rt_fail(handle)` is called immediately before the `unreachable`, so
//! the module still terminates abnormally and the host now knows why.
//!
//! ★ THE MESSAGE IS NOT HARD-CODED HERE. Each case asserts the captured text equals
//! `tw_outcome(src).error` — the reference tier's own words, computed at test time. A lock that
//! pinned string literals would pass while the two tiers drifted apart, which is the only failure
//! that matters: the browser must say what the reference tier says, not something plausible.

use std::cell::RefCell;
use std::rc::Rc;

#[path = "compile_common/mod.rs"]
mod common;

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome};

/// What the host learned: the failure message, if the module reported one before trapping.
type Reported = Rc<RefCell<Option<String>>>;

/// Read a `Text` handle out of linear memory — the host side of every Text-taking import.
/// Layout is the one every other host in this suite reads: `[len:i32, _, dataptr:i32]`.
fn read_text(c: &wasmi::Caller<'_, Reported>, handle: i32) -> String {
    let mem = c
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("module exports memory");
    let data = mem.data(c);
    let h = handle as usize;
    let len = i32::from_le_bytes(data[h..h + 4].try_into().unwrap()) as usize;
    let dptr = i32::from_le_bytes(data[h + 8..h + 12].try_into().unwrap()) as usize;
    String::from_utf8_lossy(&data[dptr..dptr + len]).into_owned()
}

/// Run `module` under `wasmi` with a host that implements `logos_rt_fail`, returning whether
/// `main` trapped and whatever message the module reported on its way out.
fn run_capturing_failure(module: &[u8]) -> (Result<(), String>, Option<String>) {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let reported: Reported = Rc::new(RefCell::new(None));
    let mut store = wasmi::Store::new(&engine, reported.clone());
    let mut linker = wasmi::Linker::<Reported>::new(&engine);

    // The import under test.
    linker
        .func_wrap(
            "env",
            "logos_rt_fail",
            |c: wasmi::Caller<'_, Reported>, handle: i32| {
                let s = read_text(&c, handle);
                *c.data().borrow_mut() = Some(s);
            },
        )
        .unwrap();
    // The print sinks a fixture may declare before it fails. Values are discarded — this lock is
    // about the failure, and `wasm_aot_lock` already owns output parity.
    // The inline-check half: the module reports a CODE and the offending VALUE, and the host
    // renders the sentence through the SAME function the interpreter formats with. Rendering it
    // here rather than hard-coding a string is the point — a host with its own wording would
    // drift from the reference tier and this lock would not notice.
    linker
        .func_wrap(
            "env",
            "logos_rt_fail_at",
            |c: wasmi::Caller<'_, Reported>, code: i32, value: i64| {
                *c.data().borrow_mut() = Some(logicaffeine_compile::semantics::failcode::message(
                    code, value,
                ));
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_i64",
            |_c: wasmi::Caller<'_, Reported>, _v: i64| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_text",
            |_c: wasmi::Caller<'_, Reported>, _h: i32| {},
        )
        .unwrap();
    linker
        .func_wrap("env", "print_nothing", |_c: wasmi::Caller<'_, Reported>| {})
        .unwrap();

    crate::common::wasm_host::link_unmodelled_imports(&mut linker);
    let instance = linker
        .instantiate(&mut store, &m)
        .expect("every import the module declares is defined by this host")
        .start(&mut store)
        .unwrap();
    let outcome = instance
        .get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .map_err(|e| e.to_string());
    let msg = reported.borrow().clone();
    (outcome, msg)
}

/// Programs that fail at RUN time, not compile time — each for a different reason, so a fix that
/// only carried one kind of message would still go red.
const FAILING: &[(&str, &str)] = &[
    // A LITERAL out-of-range index is caught at PARSE time (LOGOS teaches 1-based indexing right
    // there), so it never reaches the backend. Each of these fails where this lock is about: at
    // RUN time, for a different reason, so a fix that carried only one kind of message stays red.
    (
        "an index past the end",
        "## Main\n    Let xs be [1, 2, 3].\n    Let i be 9.\n    Let x be item i of xs.\n",
    ),
    (
        "an index before the first",
        "## Main\n    Let xs be [1, 2, 3].\n    Let i be 0.\n    Let x be item i of xs.\n",
    ),
    (
        "division by zero",
        "## Main\n    Let z be 0.\n    Let q be 7 / z.\n",
    ),
];

/// The module still TRAPS — the documented error contract, unchanged by carrying the message.
#[test]
fn a_failing_program_still_traps() {
    for (name, src) in FAILING {
        let tw = tw_outcome(src);
        assert!(
            tw.error.is_some(),
            "fixture `{name}` must fail on the reference tier:\n{src}"
        );
        let module = compile_to_wasm(src).unwrap_or_else(|e| {
            panic!("`{name}` must COMPILE (it fails at run time): {e:?}\n{src}")
        });
        let (outcome, _) = run_capturing_failure(&module);
        assert!(
            outcome.is_err(),
            "`{name}` ran to completion where the reference tier failed ({:?}). The module must \
             still TRAP — carrying the message must not turn a failure into a success.\n{src}",
            tw.error
        );
    }
}

/// ★ THE LOAD-BEARING TEST — the host is told WHY, in the reference tier's own words.
#[test]
fn the_host_is_told_why_the_program_failed() {
    for (name, src) in FAILING {
        let expected = tw_outcome(src)
            .error
            .unwrap_or_else(|| panic!("fixture `{name}` must fail on the reference tier:\n{src}"));
        let module = compile_to_wasm(src).expect("compiles");
        let (_, reported) = run_capturing_failure(&module);
        let reported = reported.unwrap_or_else(|| {
            panic!(
                "`{name}` trapped without telling the host anything. In a browser this is \
                 `RuntimeError: unreachable` with no message, no line and no source — the \
                 compiler wrote \"{expected}\" and the wasm backend dropped it.\n{src}"
            )
        });
        assert_eq!(
            reported, expected,
            "`{name}`: the browser tier reported a DIFFERENT message than the reference tier. \
             Two tiers that both fail but say different things is the drift this lock exists to \
             catch.\n{src}"
        );
    }
}

/// A program that does NOT fail reports nothing — so the host cannot be left holding a stale
/// message from an earlier run, and a success is never dressed up as a failure.
#[test]
fn a_program_that_succeeds_reports_nothing() {
    let src = "## Main\n    Let xs be [1, 2, 3].\n    Let x be item 1 of xs.\n";
    assert_eq!(
        tw_outcome(src).error,
        None,
        "this fixture must SUCCEED on the reference tier"
    );
    let module = compile_to_wasm(src).expect("compiles");
    let (outcome, reported) = run_capturing_failure(&module);
    assert!(
        outcome.is_ok(),
        "a program that succeeds on the reference tier must not trap: {outcome:?}"
    );
    assert_eq!(
        reported, None,
        "nothing failed, so the host must have been told nothing"
    );
}

/// ★ THE TWO CLASSES OF RUNTIME FAILURE, and this one is the class this seam carries.
///
/// `Op::FailWith { msg: ConstIdx }` names a `Constant::Text` the COMPILER wrote — an undefined
/// variable, a mount selector that matches nothing, a refused construct. Forty-three sites emit
/// one. The message exists at compile time, so the backend can hand it over.
///
/// The other class is an INLINE runtime check — a bounds test, a division guard — whose message
/// is formatted by the interpreter from a value that only exists while running (`Index 9 out of
/// bounds`). There is no constant to carry, so those still trap bare; see the fixtures above.
#[test]
fn a_compile_time_message_reaches_the_host() {
    let src = "## Main\n    Let flag be true.\n    If flag:\n        Show nope.\n";
    let expected = tw_outcome(src)
        .error
        .expect("an unbound name must fail at run time");
    let module = compile_to_wasm(src).expect("compiles — it fails at run time");
    let (outcome, reported) = run_capturing_failure(&module);
    assert!(outcome.is_err(), "it must still trap");
    assert_eq!(
        reported.as_deref(),
        Some(expected.as_str()),
        "a failure whose message the compiler already holds must reach the host verbatim"
    );
}
