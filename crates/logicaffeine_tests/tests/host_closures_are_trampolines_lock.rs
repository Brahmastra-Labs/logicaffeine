//! ★ LOCK — every closure a lowered page hands a host is a trampoline: it carries the instance's key
//! and nothing else. ★
//!
//! A host keeps a closure for as long as the registration lasts (a route listener, a window event,
//! a relay topic, a fetch's completion) and calls it later, after any number of turns. A closure
//! that captured a state value, or a snapshot of a global, would answer that later call with the
//! world as it was when the page registered it. So the lowering hands every host a closure whose
//! whole capture is the instance key, and the call it makes reads the instance's state where the
//! turn left it (`lgx_lower::events::host_callback_closure`).
//!
//! Two halves: a census over the compiled program of a page that registers every kind of host
//! callback, and a behaviour row: a state write after registration is what the next callback reads,
//! on the tree-walker and the VM.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// A page that registers a closure with the host for each kind of callback the lowering hands one.
const EVERY_CALLBACK: &str = "## Component App
State:
    Let seen be 0.
    Let n be 0.
    Let body be \"\".
Actions:
    When the route changes:
        Set seen to n.
    When the window resizes:
        Set seen to n.
    When the app goes offline:
        Set seen to n.
    When a message arrives on \"room\" (m: Text):
        Set seen to n.
    To load:
        Get \"/api/x\" as Text, then loaded.
    To loaded (r: Result of Text and HttpError):
        Inspect r:
            When Ok (v):
                Set body to v.
            When Err (e):
                Set body to e's message.
    To bump:
        Set n to n + 1.
View:
    <p id=\"s\">{seen}</p><p>{body}</p><button id=\"b\" on-click={bump}>b</button><button id=\"l\" on-click={load}>l</button>

## Main
Mount App.
";

/// Every closure body in `src`'s compiled program — the anonymous functions — with what each
/// captures: `(captured name, whether it is a promoted global)`.
fn closure_captures(src: &str) -> Vec<Vec<(String, bool)>> {
    let concrete = logicaffeine_compile::monomorphize::Options {
        lower_option: false,
    };
    logicaffeine_compile::ui_bridge::with_monomorphic_program_checked(
        src,
        concrete,
        |parsed, interner| {
            let (stmts, types, _) = parsed.unwrap_or_else(|e| panic!("{e:?}\n{src}"));
            let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
            let program = logicaffeine_compile::vm::Compiler::compile_with_oracle_located(
                stmts,
                interner,
                Some(types),
                Some(oracle),
            )?;
            Ok(program
                .functions
                .iter()
                .filter(|f| interner.resolve(f.name).is_empty())
                .map(|f| {
                    f.captures
                        .iter()
                        .map(|(name, global)| {
                            (interner.resolve(*name).to_string(), global.is_some())
                        })
                        .collect()
                })
                .collect())
        },
    )
    .unwrap_or_else(|e| panic!("{e:?}\n{src}"))
}

#[test]
fn every_closure_the_lowering_hands_a_host_captures_only_the_instance_key() {
    let closures = closure_captures(EVERY_CALLBACK);
    assert!(
        closures.len() >= 5,
        "the census sees the page's host closures — a route, a window resize, going offline, a relay \
         topic and a fetch completion: {closures:#?}"
    );
    for caps in &closures {
        assert!(
            caps.iter().all(|(name, global)| name == "__lgx_key" && !global),
            "a closure handed to a host captures {caps:?}: a host closure carries the instance key and \
             nothing else, so what it runs later reads the instance's state as the turns left it"
        );
    }
}

#[test]
fn a_state_write_after_registration_is_what_the_next_host_callback_reads() {
    let src = "## Component App
State:
    Let seen be 0.
    Let n be 0.
Actions:
    When the route changes:
        Set seen to n.
    To bump:
        Set n to n + 1.
    To go:
        Navigate to \"/next\".
View:
    <p id=\"s\">{seen}</p><button id=\"b\" on-click={bump}>b</button><button id=\"g\" on-click={go}>g</button>

## Main
Mount App.
Click \"#b\".
Click \"#b\".
Click \"#b\".
Click \"#g\".
";
    for (tier, run) in [
        ("tree-walker", tw_outcome as fn(&str) -> _),
        ("VM", vm_outcome),
    ] {
        dom::reset();
        let out = run(src);
        assert_eq!(out.error, None, "{tier}");
        assert!(
            dom::render().contains("<p id=\"s\">3</p>"),
            "{tier}: the route listener registered at mount reads `n` as three turns left it, not as it \
             was when it was registered: {}",
            dom::render()
        );
    }
}
