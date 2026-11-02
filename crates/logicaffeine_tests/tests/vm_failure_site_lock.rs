//! ★ LOCK — a VM runtime failure carries the `Stmt::Site` id in force at the failing pc. ★
//!
//! Wave 3: "runtime failures carry a location". `vm/instruction.rs`'s `CompiledProgram.site_at`
//! (pc → site id, sorted) is built by `vm/compiler.rs` from the `Stmt::Site` markers the parser
//! inserts before each sentence of a component's `Actions:`/handler body; `vm/machine.rs`'s
//! `Vm::failing_site()` reads it back once a run's `Err` is in hand. This is the compiler/VM half
//! only — rendering the id into `file:line:col` + component + action waits on `FailureSite::render`
//! and `ParseFacts` threading (both lang2's); this lock holds the id itself, which is already real.

use logicaffeine_compile::vm::{Compiler, Vm};

const SRC: &str = "## Component App\n\
State:\n    Let xs be [1, 2].\n\
Actions:\n    To boom:\n        Let x be 1.\n        Show item 9 of xs.\n\
View:\n    <button on-click={boom}>go</button>\n\n\
## Main\nMount App.\nClick \"button\".\n";

/// The action's SECOND sentence (`Show item 9 of xs`) is the one that fails — its own site, not the
/// first sentence's (`Let x be 1`, which never fails) or `NO_SITE` (the driving `Mount`/`Click` in
/// `## Main`, outside any action).
#[test]
fn a_vm_failure_inside_an_action_carries_a_failure_site() {
    logicaffeine_compile::ui_bridge::with_parsed_program(SRC, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("the fixture must parse");
        let program = Compiler::compile(stmts, interner).expect("the fixture must compile");
        let mut vm = Vm::new(&program);
        let result = vm.run();
        assert!(result.is_err(), "`item 9 of xs` must fail: {result:?}");
        assert!(
            result.unwrap_err().contains("out of bounds"),
            "must fail for the predicted reason (an out-of-bounds index)"
        );
        assert!(
            vm.failing_site().is_some(),
            "a runtime failure inside a component action must carry a failure-site id"
        );
    });
}

/// A failure OUTSIDE any action (the driving statements in `## Main` itself) carries no site — there
/// is no `Stmt::Site` marker at the top level, and `failing_site` must say so rather than reuse
/// whatever site happened to be active inside the action that ran just before it.
#[test]
fn a_vm_failure_outside_any_action_carries_no_site() {
    let src = "## Main\nLet xs be [1, 2].\nShow item 9 of xs.\n";
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("the fixture must parse");
        let program = Compiler::compile(stmts, interner).expect("the fixture must compile");
        let mut vm = Vm::new(&program);
        let result = vm.run();
        assert!(result.is_err(), "`item 9 of xs` must fail: {result:?}");
        assert_eq!(
            vm.failing_site(),
            None,
            "a top-level `## Main` failure carries no `Stmt::Site` marker at all"
        );
    });
}

/// The site id is per SENTENCE, not merely "some site inside this action": moving the failing
/// sentence from first to second inside the SAME action changes which id `failing_site` returns.
/// (A test that only checked `.is_some()` could not tell a real per-sentence table from one marker
/// per action, or even per component.)
fn failing_site_of(action_body: &str) -> Option<u32> {
    let src = format!(
        "## Component App\nState:\n    Let xs be [1, 2].\n\
         Actions:\n    To boom:\n{action_body}\n\
         View:\n    <button on-click={{boom}}>go</button>\n\n\
         ## Main\nMount App.\nClick \"button\".\n"
    );
    let mut site = None;
    logicaffeine_compile::semantics::dom::reset();
    logicaffeine_compile::ui_bridge::with_parsed_program(&src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("the fixture must parse");
        let program = Compiler::compile(stmts, interner).expect("the fixture must compile");
        let mut vm = Vm::new(&program);
        assert!(vm.run().is_err(), "the action must fail");
        site = vm.failing_site();
    });
    site
}

#[test]
fn the_site_id_names_the_sentence_not_just_the_action() {
    let failing_first =
        failing_site_of("        Show item 9 of xs.\n        Show \"unreached\".\n");
    let failing_second = failing_site_of("        Show \"reached\".\n        Show item 9 of xs.\n");
    let (Some(first), Some(second)) = (failing_first, failing_second) else {
        panic!(
            "both variants must carry a site: first={failing_first:?} second={failing_second:?}"
        );
    };
    assert_ne!(
        first, second,
        "moving the failing sentence to a different position in the SAME action must change the site id"
    );
}

/// A successful run's `failing_site` is never inspected by production code (only called once an
/// `Err` is in hand) — this locks that calling it anyway on a clean run does not panic, so a future
/// caller who forgets the `Err` check fails loudly through a wrong answer, never a crash.
#[test]
fn failing_site_on_a_successful_run_does_not_panic() {
    let src = "## Main\nShow \"ok\".\n";
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("the fixture must parse");
        let program = Compiler::compile(stmts, interner).expect("the fixture must compile");
        let mut vm = Vm::new(&program);
        assert!(vm.run().is_ok());
        let _ = vm.failing_site();
    });
}
