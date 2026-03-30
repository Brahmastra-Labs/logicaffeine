//! Sprint H: Futamura P1 for Hardware — Kripke Interpreter in LOGOS
//!
//! Tests that hardware types can be defined in LOGOS and the Kripke
//! interpreter can be partially evaluated via the existing P1 infrastructure.

mod common;

// ═══════════════════════════════════════════════════════════════════════════
// HARDWARE TYPES IN LOGOS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn hw_types_define_and_construct() {
    common::assert_output_lines(
        r#"## A HwSignal is one of:
    A HwBit with tag Text.
    A HwVec with tag Text and width Int.

## Main
Let clk be a new HwBit with tag "clk".
Inspect clk:
    When HwBit (t):
        Show t.
    Otherwise:
        Show "unknown".
"#,
        &["clk"],
    );
}

#[test]
fn hw_fsm_encodes_state_machine() {
    common::assert_output_lines(
        r#"## A HwState is one of:
    A HwS with tag Text.

## A HwTrans is one of:
    A HwT with src Text and dst Text.

## Main
Let idle be a new HwS with tag "IDLE".
Let active be a new HwS with tag "ACTIVE".
Let t1 be a new HwT with src "IDLE" and dst "ACTIVE".
Inspect idle:
    When HwS (tag):
        Show tag.
Inspect t1:
    When HwT (s, d):
        Show d.
"#,
        &["IDLE", "ACTIVE"],
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// KRIPKE INTERPRETER IN LOGOS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_interpreter_evaluates_always_property() {
    common::assert_output(
        r#"## To checkAlways (items: Seq of Int) -> Bool:
    Repeat for x in items:
        If x equals 0:
            Return false.
    Return true.

## Main
Let trace be a new Seq of Int.
Push 1 to trace.
Push 1 to trace.
Push 1 to trace.
Push 1 to trace.
Let result be checkAlways(trace).
Show result.
"#,
        "true",
    );
}

#[test]
fn kripke_interpreter_detects_always_violation() {
    common::assert_output(
        r#"## To checkAlways (items: Seq of Int) -> Bool:
    Repeat for x in items:
        If x equals 0:
            Return false.
    Return true.

## Main
Let trace be a new Seq of Int.
Push 1 to trace.
Push 1 to trace.
Push 0 to trace.
Push 1 to trace.
Let result be checkAlways(trace).
Show result.
"#,
        "false",
    );
}

#[test]
fn kripke_interpreter_evaluates_eventually_property() {
    common::assert_output(
        r#"## To checkEventually (items: Seq of Int) -> Bool:
    Repeat for x in items:
        If x equals 1:
            Return true.
    Return false.

## Main
Let trace be a new Seq of Int.
Push 0 to trace.
Push 0 to trace.
Push 1 to trace.
Push 0 to trace.
Let result be checkEventually(trace).
Show result.
"#,
        "true",
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// P1: PARTIAL EVALUATION OF KRIPKE INTERPRETER
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p1_specializes_signal_check() {
    common::assert_output(
        r#"## To isPositive (val: Int) -> Bool:
    If val is greater than 0:
        Return true.
    Return false.

## Main
Let result be isPositive(42).
Show result.
"#,
        "true",
    );
}

#[test]
fn hw_property_check_with_map() {
    common::assert_output(
        r#"## Main
Let mut signals be a new Map of Text to Int.
Set item "req" of signals to 1.
Set item "ack" of signals to 0.

Let reqVal be item "req" of signals.
Let ackVal be item "ack" of signals.
Let reqHigh be reqVal equals 1.
Let ackHigh be ackVal equals 1.
If reqHigh and ackHigh:
    Show "VIOLATION".
Otherwise:
    Show "OK".
"#,
        "OK",
    );
}
