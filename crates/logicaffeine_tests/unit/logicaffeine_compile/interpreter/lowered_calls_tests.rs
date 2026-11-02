//! ★ LOCK — a host call into the tree-walker gives back EXACTLY the engine state it took. ★
//!
//! `lowered_here_session_lock` proves the behaviour a host can observe; this file reads the state
//! behind it — the call depth, the environment's frame and scope stacks, the self-tail-call
//! bookkeeping, the current failure site — after clean returns, after failures at depth, and after a
//! reload issued from inside a block, so a leak that no behaviour exposes yet is still caught.
use crate::interpreter::{Interpreter, RuntimeValue};
use crate::page_host::LoweredCalls;

const PAGE: &str = "\
## To boom -> Int:
    Let local be 5.
    Return item 9 of [local].

## To deep (n: Int) -> Int:
    Repeat for i from 1 to 3:
        If n is 0:
            Return item 9 of [1].
        Set counter to counter + deep(n - 1).
    Return 0.

## To countdown (n: Int) -> Int:
    If n is at most 0:
        Return item 9 of [n].
    Return countdown(n - 1).

## To bump:
    Set counter to counter + 1.

## To readCounter -> Int:
    Return counter.

## Main
Let counter be 1.
";

/// The engine state a host call must leave exactly as it found it.
#[derive(Debug, PartialEq, Eq)]
struct Resting {
    call_depth: usize,
    frames: usize,
    scopes: usize,
    tco: bool,
    pending_tail_call: bool,
    repeat_depth: usize,
    site: Option<u32>,
}

fn resting(interp: &Interpreter<'_>) -> Resting {
    Resting {
        call_depth: interp.task.call_depth,
        frames: interp.task.env.frame_stack.len(),
        scopes: interp.task.env.save_stack.len(),
        tco: interp.task.tco_fn_sync.is_some(),
        pending_tail_call: interp.task.pending_tail_call.is_some(),
        repeat_depth: interp.task.repeat_depth_sync,
        site: interp.current_site,
    }
}

fn with_page(f: impl for<'a> FnOnce(&mut Interpreter<'a>)) {
    crate::ui_bridge::with_parsed_program(PAGE, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("the page parses");
        let mut interp = Interpreter::new(interner)
            .with_type_registry(types)
            .with_policies(policies);
        interp.run_page_main(stmts).expect("the page's main runs");
        f(&mut interp);
    });
}

#[test]
fn a_clean_call_and_a_failed_call_both_leave_the_engine_at_rest() {
    with_page(|interp| {
        let before = resting(interp);
        assert_eq!(
            before,
            Resting {
                call_depth: 0,
                frames: 0,
                scopes: 0,
                tco: false,
                pending_tail_call: false,
                repeat_depth: 0,
                site: None,
            },
            "the page's main leaves the engine at rest"
        );
        interp.call("bump", vec![]).unwrap();
        assert_eq!(
            resting(interp),
            before,
            "a clean call gives back what it took"
        );
        assert!(interp.call("boom", vec![]).is_err());
        assert_eq!(
            resting(interp),
            before,
            "a failed call gives back what it took"
        );
        assert!(interp.call("deep", vec![RuntimeValue::Int(3)]).is_err());
        assert_eq!(
            resting(interp),
            before,
            "a failure three calls deep inside loops unwinds all of it"
        );
        assert!(interp
            .call("countdown", vec![RuntimeValue::Int(50)])
            .is_err());
        assert_eq!(
            resting(interp),
            before,
            "a failure at the end of a self-tail-call loop unwinds too"
        );
    });
}

#[test]
fn a_host_call_restores_the_failure_site_it_was_entered_at() {
    with_page(|interp| {
        interp.current_site = Some(7);
        assert!(interp.call("boom", vec![]).is_err());
        assert_eq!(
            interp.current_site,
            Some(7),
            "a host call is its own entry, and leaves the site as it was"
        );
        interp.call("bump", vec![]).unwrap();
        assert_eq!(interp.current_site, Some(7));
    });
}

#[test]
fn a_reload_from_inside_a_block_rebuilds_the_page_globals_not_the_blocks_bindings() {
    with_page(|interp| {
        let counter = interp.ctx.interner.lookup("counter").expect("interned");
        interp.call("bump", vec![]).unwrap();
        interp.task.env.push_scope();
        interp.reload().expect("the page reloads");
        assert_eq!(
            interp.task.env.save_stack.len(),
            1,
            "the caller's open block is still open"
        );
        assert!(
            interp.task.env.save_stack[0].is_empty(),
            "the page's main recorded nothing into the caller's block"
        );
        assert!(
            !interp.task.env.main_block.contains_key(&counter),
            "the page's global is not block-scoped"
        );
        assert!(
            matches!(
                interp.task.env.globals.get(&counter),
                Some(RuntimeValue::Int(1))
            ),
            "the page's global starts over: {:?}",
            interp
                .task
                .env
                .globals
                .get(&counter)
                .map(|v| v.to_display_string())
        );
        interp.task.env.pop_scope();
        assert_eq!(resting(interp).scopes, 0);
    });
}

#[test]
fn a_reload_from_inside_a_function_frame_runs_at_the_pages_top_level() {
    with_page(|interp| {
        let counter = interp.ctx.interner.lookup("counter").expect("interned");
        interp.task.env.push_frame();
        interp.task.env.define(counter, RuntimeValue::Int(-5));
        interp.reload().expect("the page reloads");
        assert!(
            matches!(
                interp.task.env.globals.get(&counter),
                Some(RuntimeValue::Int(1))
            ),
            "the reload wrote the page's global, not the frame's local"
        );
        assert!(
            matches!(
                interp.task.env.locals.get(&counter),
                Some(RuntimeValue::Int(-5))
            ),
            "and the frame it was called from is intact"
        );
        interp.task.env.pop_frame();
        assert_eq!(resting(interp).frames, 0);
    });
}
