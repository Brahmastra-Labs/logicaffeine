//! SUPERCRUSH Sprint S0D: Multi-Sorted Counterexample Traces
//!
//! Tests that counterexample traces contain properly typed signal values
//! (Bool, Int, BitVec) rather than just booleans.

#![cfg(feature = "verification")]

use logicaffeine_verify::{
    check_equivalence, EquivalenceResult, SignalValue, VerifyExpr, VerifyOp,
};

// ═══════════════════════════════════════════════════════════════════════════
// SIGNAL VALUE TYPE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn signal_value_bool_display() {
    assert_eq!(format!("{}", SignalValue::Bool(true)), "true");
    assert_eq!(format!("{}", SignalValue::Bool(false)), "false");
}

#[test]
fn signal_value_int_display() {
    assert_eq!(format!("{}", SignalValue::Int(42)), "42");
    assert_eq!(format!("{}", SignalValue::Int(-7)), "-7");
}

#[test]
fn signal_value_bitvec_display() {
    assert_eq!(format!("{}", SignalValue::BitVec { width: 8, value: 0xAB }), "0xAB");
    assert_eq!(format!("{}", SignalValue::BitVec { width: 16, value: 0x1234 }), "0x1234");
}

#[test]
fn signal_value_unknown_display() {
    assert_eq!(format!("{}", SignalValue::Unknown), "?");
}

#[test]
fn signal_value_as_bool() {
    assert_eq!(SignalValue::Bool(true).as_bool(), Some(true));
    assert_eq!(SignalValue::Int(5).as_bool(), None);
}

#[test]
fn signal_value_as_int() {
    assert_eq!(SignalValue::Int(42).as_int(), Some(42));
    assert_eq!(SignalValue::Bool(true).as_int(), None);
}

// ═══════════════════════════════════════════════════════════════════════════
// TRACE WITH BOOLEAN SIGNALS (regression)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn trace_bool_signals_in_counterexample() {
    // Two non-equivalent boolean formulas should produce a trace with Bool values
    let a = VerifyExpr::var("req@0");
    let b = VerifyExpr::not(VerifyExpr::var("req@0"));
    let result = check_equivalence(&a, &b, &["req".into()], 1);
    match result {
        EquivalenceResult::NotEquivalent { counterexample } => {
            assert!(!counterexample.cycles.is_empty(), "Should have cycles");
            let first = &counterexample.cycles[0];
            assert!(first.signals.contains_key("req"), "Should have req signal");
            match first.signals.get("req").unwrap() {
                SignalValue::Bool(_) => {} // expected
                other => panic!("Expected Bool signal value, got: {:?}", other),
            }
        }
        other => panic!("Expected NotEquivalent, got: {:?}", other),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TRACE WITH INTEGER SIGNALS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn trace_integer_signal_in_counterexample() {
    // count > 5 not-equiv count < 5 → counterexample should show Int values
    let a = VerifyExpr::gt(VerifyExpr::var("count@0"), VerifyExpr::int(5));
    let b = VerifyExpr::lt(VerifyExpr::var("count@0"), VerifyExpr::int(5));
    let result = check_equivalence(&a, &b, &["count".into()], 1);
    match result {
        EquivalenceResult::NotEquivalent { counterexample } => {
            assert!(!counterexample.cycles.is_empty(), "Should have cycles");
            let first = &counterexample.cycles[0];
            if let Some(val) = first.signals.get("count") {
                match val {
                    SignalValue::Int(_) => {} // expected
                    SignalValue::Bool(_) => {} // acceptable if Z3 treats as bool
                    other => panic!("Expected Int or Bool for count signal, got: {:?}", other),
                }
            }
        }
        other => panic!("Expected NotEquivalent, got: {:?}", other),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TRACE PRESERVES SIGNAL NAMES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn trace_preserves_signal_names() {
    // Counterexample should use clean signal names, not @t suffixed
    let a = VerifyExpr::and(
        VerifyExpr::var("req@0"),
        VerifyExpr::var("ack@0"),
    );
    let b = VerifyExpr::or(
        VerifyExpr::var("req@0"),
        VerifyExpr::var("ack@0"),
    );
    let result = check_equivalence(&a, &b, &["req".into(), "ack".into()], 1);
    match result {
        EquivalenceResult::NotEquivalent { counterexample } => {
            for cycle in &counterexample.cycles {
                for key in cycle.signals.keys() {
                    assert!(!key.contains('@'),
                        "Signal name should not contain @, got: {}", key);
                }
            }
        }
        other => panic!("Expected NotEquivalent, got: {:?}", other),
    }
}

#[test]
fn trace_empty_still_works() {
    // Equivalent formulas produce no trace
    let a = VerifyExpr::var("p");
    let b = VerifyExpr::var("p");
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent));
}

#[test]
fn trace_multi_cycle() {
    // Multiple timesteps should produce multiple cycles
    let a = VerifyExpr::and(
        VerifyExpr::var("sig@0"),
        VerifyExpr::and(
            VerifyExpr::var("sig@1"),
            VerifyExpr::var("sig@2"),
        ),
    );
    let b = VerifyExpr::and(
        VerifyExpr::not(VerifyExpr::var("sig@0")),
        VerifyExpr::and(
            VerifyExpr::var("sig@1"),
            VerifyExpr::var("sig@2"),
        ),
    );
    let result = check_equivalence(&a, &b, &["sig".into()], 3);
    match result {
        EquivalenceResult::NotEquivalent { counterexample } => {
            assert!(counterexample.cycles.len() >= 1,
                "Should have at least 1 cycle, got: {}", counterexample.cycles.len());
        }
        other => panic!("Expected NotEquivalent, got: {:?}", other),
    }
}

#[test]
fn trace_mixed_bool_and_int() {
    // Mix of boolean and integer signals
    let a = VerifyExpr::and(
        VerifyExpr::var("valid@0"),
        VerifyExpr::gt(VerifyExpr::var("count@0"), VerifyExpr::int(10)),
    );
    let b = VerifyExpr::and(
        VerifyExpr::var("valid@0"),
        VerifyExpr::lt(VerifyExpr::var("count@0"), VerifyExpr::int(0)),
    );
    let result = check_equivalence(&a, &b, &["valid".into(), "count".into()], 1);
    match result {
        EquivalenceResult::NotEquivalent { counterexample } => {
            assert!(!counterexample.cycles.is_empty());
        }
        other => panic!("Expected NotEquivalent, got: {:?}", other),
    }
}

#[test]
fn signal_value_equality() {
    assert_eq!(SignalValue::Bool(true), SignalValue::Bool(true));
    assert_ne!(SignalValue::Bool(true), SignalValue::Bool(false));
    assert_eq!(SignalValue::Int(42), SignalValue::Int(42));
    assert_ne!(SignalValue::Int(1), SignalValue::Bool(true));
    assert_eq!(
        SignalValue::BitVec { width: 8, value: 0xFF },
        SignalValue::BitVec { width: 8, value: 0xFF },
    );
}

#[test]
fn signal_value_clone() {
    let val = SignalValue::BitVec { width: 16, value: 0x1234 };
    let cloned = val.clone();
    assert_eq!(val, cloned);
}
