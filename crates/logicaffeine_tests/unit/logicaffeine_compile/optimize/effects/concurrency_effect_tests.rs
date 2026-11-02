//! A concurrency / networking construct is an effectful, often nondeterministic
//! boundary. The partial evaluator's specialization gate (`partial_eval.rs`) keys
//! off these effects; if any of them are misclassified as Pure, a function holding
//! one can be specialized and a static parameter flowing into it dropped, leaving a
//! dangling free variable. These tests pin the classification at the source.

use super::EffectEnv;

#[test]
fn effects_select_is_not_pure() {
    // A `Select` is a nondeterministic choice — never Pure. The classifier must also
    // see *into* the branch bodies (the `Show` here is an effect of the function).
    let src = "\
## To sel (ch: Int):
    Await the first of:
        Receive x from ch:
            Show x.
        After 1 seconds:
            Show 0.

## Main
    Let ch be a Pipe of Int.
    Launch a task to sel with ch.
";
    let env = EffectEnv::analyze_source(src).expect("parse");
    assert!(
        !env.function_is_pure("sel"),
        "a function whose body is a Select must not be classified Pure"
    );
}

#[test]
fn effects_launch_task_not_pure() {
    let src = "\
## To helper (ch: Int):
    Receive y from ch.
    Show y.

## To launcher (ch: Int):
    Launch a task to helper with ch.

## Main
    Let ch be a Pipe of Int.
    Launch a task to launcher with ch.
";
    let env = EffectEnv::analyze_source(src).expect("parse");
    assert!(
        !env.function_is_pure("launcher"),
        "a function that launches a task is concurrent, not Pure"
    );
}

#[test]
fn effects_try_receive_nondet() {
    let src = "\
## To tryer (ch: Int):
    Try to receive v from ch.

## Main
    Let ch be a Pipe of Int.
    Launch a task to tryer with ch.
";
    let env = EffectEnv::analyze_source(src).expect("parse");
    assert!(
        !env.function_is_pure("tryer"),
        "a non-blocking receive depends on instantaneous buffer state — not Pure"
    );
}

#[test]
fn effects_mount_writes_var() {
    let src = "\
## Main
    Mount counter at \"data/counter.journal\".
    Show counter.
";
    let env = EffectEnv::analyze_source(src).expect("parse");
    assert!(
        env.has_write_to("counter"),
        "Mount binds its variable — the write must be recorded"
    );
}
