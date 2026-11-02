use super::*;
use std::cell::Cell;
use std::sync::{Arc, Mutex};

/// The panic hook is the process's: the tests that replace it take turns.
static HOOK: Mutex<()> = Mutex::new(());

#[test]
fn nothing_is_caught_outside_a_boundary() {
    assert!(!is_caught());
}

#[test]
fn a_boundary_catches_for_as_long_as_it_runs_and_no_longer() {
    let inside = caught(|| {
        let nested = caught(is_caught).expect("the nested call returns");
        (nested, is_caught())
    })
    .expect("the call returns");
    assert_eq!(inside, (true, true), "nested and after the nested boundary returns");
    assert!(!is_caught(), "after the boundary returns");
}

#[test]
fn a_caught_panic_answers_err_and_leaves_the_thread_uncaught() {
    let _turn = HOOK.lock().unwrap_or_else(|p| p.into_inner());
    let previous = Arc::new(std::panic::take_hook());
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = caught(|| -> u8 { panic!("inside the boundary") });
    let nested = caught(|| caught(|| -> u8 { panic!("inside two") }).is_err());
    std::panic::set_hook(Box::new(move |info| previous(info)));
    assert!(outcome.is_err(), "the panic ends at the boundary");
    assert_eq!(nested.ok(), Some(true), "the inner boundary catches, the outer returns");
    assert!(!is_caught(), "unwinding out of a boundary leaves it");
}

#[test]
fn a_panic_hook_sees_the_boundary_that_will_catch_the_panic() {
    thread_local! {
        static SEEN: Cell<Option<bool>> = const { Cell::new(None) };
    }
    let _turn = HOOK.lock().unwrap_or_else(|p| p.into_inner());
    let previous = Arc::new(std::panic::take_hook());
    std::panic::set_hook(Box::new(|_| SEEN.with(|s| s.set(Some(is_caught())))));
    let _ = caught(|| -> u8 { panic!("inside the boundary") });
    let inside = SEEN.with(|s| s.take());
    let outside = std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(|| -> u8 { panic!("no boundary") });
        SEEN.with(|s| s.take())
    })
    .join()
    .expect("the thread returns");
    std::panic::set_hook(Box::new(move |info| previous(info)));
    assert_eq!(inside, Some(true), "a panic inside `caught` is caught");
    assert_eq!(outside, Some(false), "a panic outside every boundary ends the program");
}
