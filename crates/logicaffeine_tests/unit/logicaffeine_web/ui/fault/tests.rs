use super::*;
use std::panic::{catch_unwind, panic_any, AssertUnwindSafe};

/// `contain`, under a net of the test's own: a panic that got past the boundary would
/// otherwise end the test before it could say so.
fn contained<T>(work: impl FnOnce() -> T) -> Result<T, Fault> {
    catch_unwind(AssertUnwindSafe(|| contain(work))).unwrap_or_else(|_| panic!("the panic escaped the boundary"))
}

fn fault_of<T: std::fmt::Debug>(work: impl FnOnce() -> T) -> Fault {
    contained(work).expect_err("the work was supposed to fault")
}

#[test]
fn work_that_finishes_gives_back_its_value() {
    assert_eq!(contain(|| 6 * 7), Ok(42));
    assert_eq!(contain(|| vec!["kernel".to_string(); 3]), Ok(vec!["kernel".to_string(); 3]));
}

#[test]
fn a_panic_becomes_a_fault_with_its_message() {
    let fault = fault_of(|| -> () { panic!("the kernel lost its place") });
    assert_eq!(fault.cause, Cause::Panic);
    assert_eq!(fault.message, "the kernel lost its place");
}

#[test]
fn a_fault_names_where_the_panic_was_raised() {
    let line = line!() + 1;
    let fault = fault_of(|| -> () { panic!("here") });
    let location = fault.location.expect("a panic has a location");
    assert!(location.starts_with(file!()), "the location names another file: {location}");
    assert!(location.contains(&format!(":{line}:")), "the location names another line: {location}");
}

#[test]
fn a_formatted_panic_keeps_its_whole_message() {
    let (want, found) = (3, 5);
    let fault = fault_of(|| -> () { panic!("expected {want} arguments but found {found}") });
    assert_eq!(fault.message, "expected 3 arguments but found 5");
}

#[test]
fn a_failed_assertion_keeps_every_line() {
    let fault = fault_of(|| assert_eq!(1 + 1, 3, "arithmetic"));
    assert_eq!(fault.cause, Cause::Panic);
    assert!(fault.message.contains("arithmetic"), "the assertion's own words were lost: {:?}", fault.message);
    assert!(fault.message.contains("left: 2"), "the left side was lost: {:?}", fault.message);
    assert!(fault.message.contains("right: 3"), "the right side was lost: {:?}", fault.message);
}

#[test]
fn a_panic_with_no_text_says_it_carried_none() {
    let fault = fault_of(|| -> () { panic_any(42u8) });
    assert_eq!(fault.cause, Cause::Panic);
    assert_eq!(fault.message, "a panic that carried no message");
}

#[test]
fn a_fault_inside_a_fault_boundary_stays_inside() {
    let outer = contained(|| contain(|| -> () { panic!("inner") }));
    let inner = outer.expect("the outer boundary saw the inner fault").expect_err("the inner work faulted");
    assert_eq!(inner.message, "inner");
    assert_eq!(contain(|| "after"), Ok("after"), "the next work inherited the inner fault");
}

#[test]
fn a_panic_the_work_handles_itself_is_no_fault() {
    let handled = contained(|| catch_unwind(|| -> () { panic!("handled") }).is_err());
    assert_eq!(handled, Ok(true));
    let next = fault_of(|| -> () { panic!("the next one") });
    assert_eq!(next.message, "the next one", "the handled panic was reported as a later fault");
}

#[test]
fn the_boundary_can_be_crossed_again_and_again() {
    for round in 0..200 {
        let fault = fault_of(|| -> () { panic!("round {round}") });
        assert_eq!(fault.message, format!("round {round}"));
        assert_eq!(contain(|| round), Ok(round));
    }
}

fn panicked(message: &str, location: Option<&str>) -> Fault {
    Fault { cause: Cause::Panic, message: message.to_string(), location: location.map(str::to_string) }
}

#[test]
fn the_headline_names_the_action_and_the_first_line_of_the_fault() {
    let stopped = ActionFault { action: "Execute", fault: panicked("index out of bounds\nsecond line", Some("kernel.rs:1:2")) };
    assert_eq!(stopped.headline(), "Execute failed: index out of bounds");
}

#[test]
fn running_out_of_stack_reads_as_such() {
    let fault = Fault::stack_exhausted();
    assert_eq!(fault.cause, Cause::StackExhausted);
    assert_eq!(ActionFault { action: "Compile", fault }.headline(), "Compile failed: ran out of stack space");
}

#[test]
fn the_detail_carries_the_whole_fault_and_says_the_page_lives_on() {
    let stopped = ActionFault { action: "Auto", fault: panicked("first\nsecond", Some("prover.rs:10:5")) };
    let detail = stopped.detail();
    assert!(detail.starts_with("Auto failed unexpectedly: first\nsecond"), "{detail:?}");
    assert!(detail.contains("at prover.rs:10:5"), "the place is missing: {detail:?}");
    assert!(detail.contains("still running"), "it does not say the Studio lives on: {detail:?}");
}

#[test]
fn a_fault_with_no_place_leaves_the_place_out() {
    let detail = ActionFault { action: "Execute", fault: Fault::stack_exhausted() }.detail();
    assert!(!detail.contains("\nat "), "a fault with no location still names one: {detail:?}");
    assert!(detail.contains("ran out of stack space"), "{detail:?}");
}
