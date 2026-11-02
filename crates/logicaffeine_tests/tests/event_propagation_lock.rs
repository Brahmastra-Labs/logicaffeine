//! ★ LOCK — an event travels an in-process page the way it travels a browser's.
//!
//! A browser dispatches an event along a path: the capture-phase listeners of the target's ancestors
//! from the root down, the target's own listener, then — for an event that bubbles — the other
//! ancestors' listeners from the target back up. A listener registered with `stop` ends the walk
//! after it runs; one registered `passive` cannot cancel the event's default action; `once` is the
//! lowering's own bookkeeping and never the host's. `semantics::dom::dispatch_path` is that walk for
//! every in-process tier, and `browser_tier_lock` holds it to Chrome.
//!
//! | test | property |
//! |---|---|
//! | `a_bubbling_event_reaches_the_target_then_its_listening_ancestors_nearest_first` | bubble order |
//! | `the_target_is_always_on_the_path` | the target's own listener decides whether the target answers |
//! | `capture_listeners_run_on_the_way_down_before_the_target` | capture order, and a capture listener is not visited again going up |
//! | `stop_ends_the_walk_after_the_listener_that_stops_it` | at a capture ancestor, at the target, on the way up |
//! | `an_event_that_does_not_bubble_reaches_capture_listeners_and_the_target_only` | focus/blur |
//! | `the_default_action_is_cancelled_only_by_a_reached_listener_that_is_not_passive` | prevent × passive × stop |
//! | `once_is_never_the_hosts` | the ONCE bit changes no walk |
//! | `a_node_the_page_no_longer_holds_is_never_on_a_path` | detached ancestors |
//! | `listen_flags_registers_the_same_listener_on_the_tree_walker_and_the_vm` | the builtin, both engines |

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{
    self, DomOp, LISTEN_CAPTURE, LISTEN_ONCE, LISTEN_PASSIVE, LISTEN_PREVENT, LISTEN_STOP,
};

/// `body > div#outer > section#middle > button#target`, in a fresh document.
fn nested() -> (i64, i64, i64, i64) {
    dom::reset();
    let body = dom::body();
    let outer = dom::create_element("div");
    dom::set_attribute(outer, "id", "outer");
    let middle = dom::create_element("section");
    dom::set_attribute(middle, "id", "middle");
    let target = dom::create_element("button");
    dom::set_attribute(target, "id", "target");
    dom::append_child(body, outer);
    dom::append_child(outer, middle);
    dom::append_child(middle, target);
    (body, outer, middle, target)
}

/// The nodes `event` at `target` reaches, in order, as the page stands.
fn walk(target: i64, event: &str) -> Vec<i64> {
    dom::dispatch_path(target, event)
        .stops
        .iter()
        .map(|s| dom::query_selector(s))
        .collect()
}

#[test]
fn a_bubbling_event_reaches_the_target_then_its_listening_ancestors_nearest_first() {
    let (body, outer, middle, target) = nested();
    dom::listen(outer, "click", 0);
    dom::listen(body, "click", 0);
    dom::listen(target, "click", 0);
    assert_eq!(
        walk(target, "click"),
        vec![target, outer, body],
        "the section has no listener and is passed by"
    );
    dom::listen(middle, "click", 0);
    assert_eq!(walk(target, "click"), vec![target, middle, outer, body]);
    assert_eq!(
        walk(target, "keydown"),
        vec![target],
        "a listener hears only its own event"
    );
}

#[test]
fn the_target_is_always_on_the_path() {
    let (_, outer, _, target) = nested();
    dom::listen(outer, "click", 0);
    assert_eq!(
        walk(target, "click"),
        vec![target, outer],
        "the target is where the event happens; whether anything there answers is the page's own \
         dispatcher's to say"
    );
}

#[test]
fn capture_listeners_run_on_the_way_down_before_the_target() {
    let (body, outer, middle, target) = nested();
    dom::listen(body, "click", 0);
    dom::listen(outer, "click", LISTEN_CAPTURE);
    dom::listen(middle, "click", 0);
    assert_eq!(
        walk(target, "click"),
        vec![outer, target, middle, body],
        "a capture listener runs before the target, and is not visited again on the way up"
    );
    dom::listen(body, "click", LISTEN_CAPTURE);
    assert_eq!(
        walk(target, "click"),
        vec![body, outer, target, middle],
        "capture order is root first"
    );
}

#[test]
fn stop_ends_the_walk_after_the_listener_that_stops_it() {
    let (body, outer, middle, target) = nested();
    dom::listen(body, "click", 0);
    dom::listen(outer, "click", LISTEN_STOP);
    dom::listen(middle, "click", 0);
    assert_eq!(
        walk(target, "click"),
        vec![target, middle, outer],
        "on the way up: the outer div stops it"
    );
    dom::listen(target, "click", LISTEN_STOP);
    assert_eq!(
        walk(target, "click"),
        vec![target],
        "at the target: nothing above hears it"
    );
    dom::listen(target, "click", 0);
    dom::listen(outer, "click", LISTEN_CAPTURE | LISTEN_STOP);
    assert_eq!(
        walk(target, "click"),
        vec![outer],
        "at a capture ancestor: not even the target hears it"
    );
}

#[test]
fn an_event_that_does_not_bubble_reaches_capture_listeners_and_the_target_only() {
    let (body, outer, middle, target) = nested();
    for event in ["focus", "blur"] {
        assert!(!dom::event_bubbles(event), "`{event}` does not bubble");
        dom::listen(body, event, 0);
        dom::listen(middle, event, 0);
        dom::listen(outer, event, LISTEN_CAPTURE);
        assert_eq!(
            walk(target, event),
            vec![outer, target],
            "`{event}`: capture on the way down, then the target"
        );
    }
    assert!(dom::event_bubbles("click"), "a click bubbles");
}

#[test]
fn the_default_action_is_cancelled_only_by_a_reached_listener_that_is_not_passive() {
    let (_, outer, middle, target) = nested();
    dom::listen(outer, "click", LISTEN_PREVENT);
    assert!(
        dom::dispatch_path(target, "click").prevented,
        "a preventing ancestor on the path cancels it"
    );
    dom::listen(outer, "click", LISTEN_PREVENT | LISTEN_PASSIVE);
    assert!(
        !dom::dispatch_path(target, "click").prevented,
        "a passive listener promised not to cancel, and a browser holds it to that"
    );
    dom::listen(outer, "click", LISTEN_PREVENT);
    dom::listen(middle, "click", LISTEN_STOP);
    assert!(
        !dom::dispatch_path(target, "click").prevented,
        "a preventing listener the walk never reaches cancels nothing"
    );
    dom::prevent_default(target, "click");
    assert!(
        dom::dispatch_path(target, "click").prevented,
        "`preventDefault` on the target itself cancels it"
    );
}

#[test]
fn once_is_never_the_hosts() {
    let (_, outer, _, target) = nested();
    dom::listen(outer, "click", LISTEN_ONCE);
    assert_eq!(walk(target, "click"), vec![target, outer]);
    assert_eq!(
        walk(target, "click"),
        vec![target, outer],
        "a `.once` listener answers its first event by the lowering's own bookkeeping; the host keeps \
         delivering, since a key-filtered miss must not spend it"
    );
}

#[test]
fn a_node_the_page_no_longer_holds_is_never_on_a_path() {
    let (body, outer, middle, _) = nested();
    dom::listen(outer, "click", 0);
    dom::listen(middle, "click", 0);
    dom::clear_children(body);
    let fresh = dom::create_element("button");
    dom::append_child(body, fresh);
    dom::listen(body, "click", 0);
    assert_eq!(
        walk(fresh, "click"),
        vec![fresh, body],
        "the old tree's listeners are gone with it"
    );
}

const LISTENS_WITH_FLAGS: &str = "## Main\n\
    Let outer be createElement(\"div\").\n\
    Let inner be createElement(\"button\").\n\
    Call setAttribute with inner and \"id\" and \"inner\".\n\
    Let b be body().\n\
    Call appendChild with b and outer.\n\
    Call appendChild with outer and inner.\n\
    Call listenFlags with outer and \"click\" and 10.\n\
    Call listenFlags with inner and \"click\" and 1.\n";

#[test]
fn listen_flags_registers_the_same_listener_on_the_tree_walker_and_the_vm() {
    let mut logs = Vec::new();
    for (tier, run) in [
        ("the tree-walker", tw_outcome as fn(&str) -> _),
        ("the VM", vm_outcome),
    ] {
        dom::reset();
        let outcome = run(LISTENS_WITH_FLAGS);
        assert!(outcome.error.is_none(), "{tier}: {:?}", outcome.error);
        let log = dom::op_log();
        assert!(
            log.contains(&DomOp::Listen {
                node: 1,
                event: "click".into(),
                flags: 10
            }) && log.contains(&DomOp::Listen {
                node: 2,
                event: "click".into(),
                flags: 1
            }),
            "{tier}: `listenFlags` records each registration with its flags: {log:?}"
        );
        let inner = dom::query_selector("#inner");
        let path = dom::dispatch_path(inner, "click");
        assert_eq!(
            path.stops
                .iter()
                .map(|s| dom::query_selector(s))
                .collect::<Vec<_>>(),
            vec![1],
            "{tier}: the outer div listens in the capture phase and stops the click there, so the \
             button — the target — never hears it"
        );
        assert!(
            !path.prevented,
            "{tier}: the button's `.prevent` listener is never reached"
        );
        logs.push(log);
    }
    assert_eq!(
        logs[0], logs[1],
        "both engines record the same registrations"
    );
}
