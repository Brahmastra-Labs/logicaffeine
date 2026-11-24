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

#[test]
fn a_speculation_that_finishes_answers_what_it_computed() {
    assert_eq!(speculate(|| logicaffeine_data::ops::logos_add_i64(40, 2)), Some(42));
}

#[test]
fn a_speculation_that_meets_a_value_past_the_word_answers_none_and_silently() {
    thread_local! {
        static SILENT: Cell<Vec<bool>> = const { Cell::new(Vec::new()) };
    }
    let _turn = HOOK.lock().unwrap_or_else(|p| p.into_inner());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {
        let quiet = speculation_catches();
        SILENT.with(|s| {
            let mut seen = s.take();
            seen.push(quiet);
            s.set(seen);
        })
    }));
    let past = speculate(|| logicaffeine_data::ops::logos_add_i64(i64::MAX, 1));
    let narrowed = speculate(|| logicaffeine_data::ops::logos_narrow_i128(i128::from(i64::MIN) - 1));
    let exact = speculate(|| {
        logicaffeine_data::LogosInt::from_i64(i64::MAX).add(&logicaffeine_data::LogosInt::from_i64(1)).expect_i64("Int")
    });
    let other = std::panic::catch_unwind(|| speculate(|| -> u8 { panic!("not an overflow") }));
    std::panic::set_hook(previous);
    assert_eq!((past, narrowed, exact), (None, None, None), "each value past the word ends the speculation");
    assert!(other.is_err(), "any other failure unwinds past the speculation");
    assert_eq!(
        SILENT.with(|s| s.take()),
        vec![true, true, true, false],
        "the hook keeps quiet for each overflow a speculation catches, and only for those"
    );
    assert!(!logicaffeine_data::ops::overflow_raised(), "the boundary that caught the overflow cleared its mark");
}

#[test]
fn a_speculation_does_not_catch_any_other_failure() {
    let _turn = HOOK.lock().unwrap_or_else(|p| p.into_inner());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let other = std::panic::catch_unwind(|| speculate(|| -> u8 { panic!("not an overflow") }));
    std::panic::set_hook(previous);
    assert!(other.is_err(), "a failure that is no overflow unwinds past the speculation");
}

#[test]
fn a_stale_overflow_mark_never_makes_a_later_failure_a_speculation_miss() {
    let _turn = HOOK.lock().unwrap_or_else(|p| p.into_inner());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let _ = caught(|| logicaffeine_data::ops::logos_add_i64(i64::MAX, 1));
    let later = std::panic::catch_unwind(|| speculate(|| -> u8 { panic!("not an overflow") }));
    std::panic::set_hook(previous);
    assert!(later.is_err(), "the mark an earlier caught overflow left is not this failure's");
}
