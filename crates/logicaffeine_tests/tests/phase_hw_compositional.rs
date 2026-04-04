//! SUPERCRUSH Sprint S1E: Assume-Guarantee Compositional Reasoning

#![cfg(feature = "verification")]

use logicaffeine_verify::compositional::{verify_compositional, ComponentSpec, CompositionalResult};
use logicaffeine_verify::{VerifyExpr, VerifyOp};

#[test]
fn comp_single_module() {
    let comp = ComponentSpec {
        name: "producer".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("valid@t")],
        init: VerifyExpr::var("valid@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("valid@t1"), VerifyExpr::var("valid@t")),
    };
    let result = verify_compositional(&[comp]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Single verified component. Got: {:?}", result);
}

#[test]
fn comp_two_modules() {
    let producer = ComponentSpec {
        name: "producer".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("data_valid@t")],
        init: VerifyExpr::var("data_valid@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("data_valid@t1"), VerifyExpr::var("data_valid@t")),
    };
    let consumer = ComponentSpec {
        name: "consumer".into(),
        assumes: vec![VerifyExpr::var("data_valid@t")],
        guarantees: vec![VerifyExpr::var("output_ok@t")],
        init: VerifyExpr::var("output_ok@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("output_ok@t1"), VerifyExpr::var("data_valid@t")),
    };
    let result = verify_compositional(&[producer, consumer]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Producer-consumer should verify. Got: {:?}", result);
}

#[test]
fn comp_failed_component_identified() {
    let bad_comp = ComponentSpec {
        name: "broken".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("always_true@t")],
        init: VerifyExpr::not(VerifyExpr::var("always_true@0")),
        transition: VerifyExpr::bool(true),
    };
    let result = verify_compositional(&[bad_comp]);
    match result {
        CompositionalResult::ComponentFailed { name, .. } => {
            assert_eq!(name, "broken");
        }
        other => panic!("Expected ComponentFailed, got: {:?}", other),
    }
}

#[test]
fn comp_empty_components() {
    let result = verify_compositional(&[]);
    assert!(matches!(result, CompositionalResult::AllVerified));
}

#[test]
fn comp_three_chain() {
    let a = ComponentSpec {
        name: "A".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("a_out@t")],
        init: VerifyExpr::var("a_out@0"),
        transition: VerifyExpr::var("a_out@t1"),
    };
    let b = ComponentSpec {
        name: "B".into(),
        assumes: vec![VerifyExpr::var("a_out@t")],
        guarantees: vec![VerifyExpr::var("b_out@t")],
        init: VerifyExpr::var("b_out@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("b_out@t1"), VerifyExpr::var("a_out@t")),
    };
    let c = ComponentSpec {
        name: "C".into(),
        assumes: vec![VerifyExpr::var("b_out@t")],
        guarantees: vec![VerifyExpr::var("c_out@t")],
        init: VerifyExpr::var("c_out@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("c_out@t1"), VerifyExpr::var("b_out@t")),
    };
    let result = verify_compositional(&[a, b, c]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Three-chain should verify. Got: {:?}", result);
}

#[test]
fn comp_guarantee_checked() {
    // Component with guarantee that holds
    let comp = ComponentSpec {
        name: "safe".into(),
        assumes: vec![],
        guarantees: vec![
            VerifyExpr::gte(VerifyExpr::var("count@t"), VerifyExpr::int(0)),
        ],
        init: VerifyExpr::eq(VerifyExpr::var("count@0"), VerifyExpr::int(0)),
        transition: VerifyExpr::eq(
            VerifyExpr::var("count@t1"),
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("count@t"), VerifyExpr::int(1)),
        ),
    };
    let result = verify_compositional(&[comp]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Counter >= 0 should verify. Got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// EXTENDED COMPOSITIONAL TESTS — SPEC DELTA (14 additional)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn comp_interface_mismatch() {
    // B assumes data_valid, but A doesn't guarantee it
    let a = ComponentSpec {
        name: "A".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("clock@t")], // guarantees clock, not data_valid
        init: VerifyExpr::var("clock@0"),
        transition: VerifyExpr::var("clock@t1"),
    };
    let b = ComponentSpec {
        name: "B".into(),
        assumes: vec![VerifyExpr::var("data_valid@t")], // needs data_valid
        guarantees: vec![VerifyExpr::var("output@t")],
        init: VerifyExpr::var("output@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("output@t1"), VerifyExpr::var("data_valid@t")),
    };
    let result = verify_compositional(&[a, b]);
    // This should either fail or detect unmet assumption
    assert!(!matches!(result, CompositionalResult::AllVerified),
        "Unmet assumption should not verify. Got: {:?}", result);
}

#[test]
fn comp_mutual_dependency() {
    // A assumes B's output, B assumes A's output — circular
    let a = ComponentSpec {
        name: "A".into(),
        assumes: vec![VerifyExpr::var("b_out@t")],
        guarantees: vec![VerifyExpr::var("a_out@t")],
        init: VerifyExpr::var("a_out@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("a_out@t1"), VerifyExpr::var("b_out@t")),
    };
    let b = ComponentSpec {
        name: "B".into(),
        assumes: vec![VerifyExpr::var("a_out@t")],
        guarantees: vec![VerifyExpr::var("b_out@t")],
        init: VerifyExpr::var("b_out@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("b_out@t1"), VerifyExpr::var("a_out@t")),
    };
    let result = verify_compositional(&[a, b]);
    // Circular dependency with matching inits should still verify
    assert!(matches!(result, CompositionalResult::AllVerified | CompositionalResult::CircularDependency { .. }),
        "Mutual dependency should handle gracefully. Got: {:?}", result);
}

#[test]
fn comp_multiple_guarantees() {
    let comp = ComponentSpec {
        name: "multi".into(),
        assumes: vec![],
        guarantees: vec![
            VerifyExpr::var("a@t"),
            VerifyExpr::var("b@t"),
        ],
        init: VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0")),
        transition: VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
            VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
        ),
    };
    let result = verify_compositional(&[comp]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Component with multiple guarantees should verify. Got: {:?}", result);
}

#[test]
fn comp_multiple_assumptions() {
    let a = ComponentSpec {
        name: "A".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("x@t"), VerifyExpr::var("y@t")],
        init: VerifyExpr::and(VerifyExpr::var("x@0"), VerifyExpr::var("y@0")),
        transition: VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("x@t1"), VerifyExpr::var("x@t")),
            VerifyExpr::iff(VerifyExpr::var("y@t1"), VerifyExpr::var("y@t")),
        ),
    };
    let b = ComponentSpec {
        name: "B".into(),
        assumes: vec![VerifyExpr::var("x@t"), VerifyExpr::var("y@t")],
        guarantees: vec![VerifyExpr::var("z@t")],
        init: VerifyExpr::var("z@0"),
        transition: VerifyExpr::iff(
            VerifyExpr::var("z@t1"),
            VerifyExpr::and(VerifyExpr::var("x@t"), VerifyExpr::var("y@t")),
        ),
    };
    let result = verify_compositional(&[a, b]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Multiple assumptions met should verify. Got: {:?}", result);
}

#[test]
fn comp_four_component_chain() {
    let make_link = |name: &str, input: &str, output: &str| -> ComponentSpec {
        ComponentSpec {
            name: name.into(),
            assumes: if input.is_empty() { vec![] } else { vec![VerifyExpr::var(&format!("{}@t", input))] },
            guarantees: vec![VerifyExpr::var(&format!("{}@t", output))],
            init: VerifyExpr::var(&format!("{}@0", output)),
            transition: if input.is_empty() {
                VerifyExpr::var(&format!("{}@t1", output))
            } else {
                VerifyExpr::iff(
                    VerifyExpr::var(&format!("{}@t1", output)),
                    VerifyExpr::var(&format!("{}@t", input)),
                )
            },
        }
    };
    let result = verify_compositional(&[
        make_link("src", "", "d1"),
        make_link("stage1", "d1", "d2"),
        make_link("stage2", "d2", "d3"),
        make_link("sink", "d3", "d4"),
    ]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "4-component chain should verify. Got: {:?}", result);
}

#[test]
fn comp_integer_interface() {
    let producer = ComponentSpec {
        name: "counter".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::gte(VerifyExpr::var("count@t"), VerifyExpr::int(0))],
        init: VerifyExpr::eq(VerifyExpr::var("count@0"), VerifyExpr::int(0)),
        transition: VerifyExpr::eq(
            VerifyExpr::var("count@t1"),
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("count@t"), VerifyExpr::int(1)),
        ),
    };
    let consumer = ComponentSpec {
        name: "checker".into(),
        assumes: vec![VerifyExpr::gte(VerifyExpr::var("count@t"), VerifyExpr::int(0))],
        guarantees: vec![VerifyExpr::var("ok@t")],
        init: VerifyExpr::var("ok@0"),
        transition: VerifyExpr::iff(
            VerifyExpr::var("ok@t1"),
            VerifyExpr::gte(VerifyExpr::var("count@t"), VerifyExpr::int(0)),
        ),
    };
    let result = verify_compositional(&[producer, consumer]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Integer interface should verify. Got: {:?}", result);
}

#[test]
fn comp_broken_guarantee() {
    // Component claims guarantee it can't keep
    let comp = ComponentSpec {
        name: "liar".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("always_true@t")],
        init: VerifyExpr::var("always_true@0"),
        transition: VerifyExpr::not(VerifyExpr::var("always_true@t1")), // breaks guarantee
    };
    let result = verify_compositional(&[comp]);
    assert!(matches!(result, CompositionalResult::ComponentFailed { .. }),
        "Broken guarantee should be detected. Got: {:?}", result);
}

#[test]
fn comp_no_assumptions() {
    // All components self-sufficient
    let a = ComponentSpec {
        name: "A".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("a@t")],
        init: VerifyExpr::var("a@0"),
        transition: VerifyExpr::var("a@t1"),
    };
    let b = ComponentSpec {
        name: "B".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("b@t")],
        init: VerifyExpr::var("b@0"),
        transition: VerifyExpr::var("b@t1"),
    };
    let result = verify_compositional(&[a, b]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Self-sufficient components should verify. Got: {:?}", result);
}

#[test]
fn comp_deterministic() {
    let comp = ComponentSpec {
        name: "det".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("p@t")],
        init: VerifyExpr::var("p@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t")),
    };
    let r1 = verify_compositional(&[comp.clone()]);
    let r2 = verify_compositional(&[comp]);
    let both_ok = matches!(r1, CompositionalResult::AllVerified)
        && matches!(r2, CompositionalResult::AllVerified);
    assert!(both_ok, "Same input should give same result");
}

#[test]
fn comp_arbiter_clients() {
    // Arbiter with 2 clients
    let arbiter = ComponentSpec {
        name: "arbiter".into(),
        assumes: vec![],
        guarantees: vec![
            VerifyExpr::not(VerifyExpr::and(
                VerifyExpr::var("g1@t"),
                VerifyExpr::var("g2@t"),
            )),
        ],
        init: VerifyExpr::and(
            VerifyExpr::not(VerifyExpr::var("g1@0")),
            VerifyExpr::not(VerifyExpr::var("g2@0")),
        ),
        transition: VerifyExpr::not(VerifyExpr::and(
            VerifyExpr::var("g1@t1"),
            VerifyExpr::var("g2@t1"),
        )),
    };
    let result = verify_compositional(&[arbiter]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Mutex arbiter should verify. Got: {:?}", result);
}

#[test]
fn comp_implication_guarantee() {
    // Guarantee: req → grant
    let comp = ComponentSpec {
        name: "handler".into(),
        assumes: vec![],
        guarantees: vec![
            VerifyExpr::implies(VerifyExpr::var("req@t"), VerifyExpr::var("grant@t")),
        ],
        init: VerifyExpr::and(VerifyExpr::var("req@0"), VerifyExpr::var("grant@0")),
        transition: VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("req@t1"), VerifyExpr::var("req@t")),
            VerifyExpr::iff(VerifyExpr::var("grant@t1"), VerifyExpr::var("req@t")),
        ),
    };
    let result = verify_compositional(&[comp]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "req→grant with grant following req should verify. Got: {:?}", result);
}

#[test]
fn comp_five_components() {
    // 5 independent verified components
    let comps: Vec<ComponentSpec> = (0..5).map(|i| {
        let sig = format!("s{}@t", i);
        let sig0 = format!("s{}@0", i);
        let sig1 = format!("s{}@t1", i);
        ComponentSpec {
            name: format!("c{}", i),
            assumes: vec![],
            guarantees: vec![VerifyExpr::var(&sig)],
            init: VerifyExpr::var(&sig0),
            transition: VerifyExpr::iff(VerifyExpr::var(&sig1), VerifyExpr::var(&sig)),
        }
    }).collect();
    let result = verify_compositional(&comps);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "5 independent components should verify. Got: {:?}", result);
}

#[test]
fn comp_temporal_guarantee() {
    // Guarantee is temporal: always p
    let comp = ComponentSpec {
        name: "temporal".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("p@t")],
        init: VerifyExpr::var("p@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t")),
    };
    let result = verify_compositional(&[comp]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Temporal guarantee should verify. Got: {:?}", result);
}

#[test]
fn comp_shared_signal() {
    // Two components read same signal
    let source = ComponentSpec {
        name: "source".into(),
        assumes: vec![],
        guarantees: vec![VerifyExpr::var("shared@t")],
        init: VerifyExpr::var("shared@0"),
        transition: VerifyExpr::var("shared@t1"),
    };
    let reader1 = ComponentSpec {
        name: "reader1".into(),
        assumes: vec![VerifyExpr::var("shared@t")],
        guarantees: vec![VerifyExpr::var("r1@t")],
        init: VerifyExpr::var("r1@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("r1@t1"), VerifyExpr::var("shared@t")),
    };
    let reader2 = ComponentSpec {
        name: "reader2".into(),
        assumes: vec![VerifyExpr::var("shared@t")],
        guarantees: vec![VerifyExpr::var("r2@t")],
        init: VerifyExpr::var("r2@0"),
        transition: VerifyExpr::iff(VerifyExpr::var("r2@t1"), VerifyExpr::var("shared@t")),
    };
    let result = verify_compositional(&[source, reader1, reader2]);
    assert!(matches!(result, CompositionalResult::AllVerified),
        "Shared signal should verify. Got: {:?}", result);
}
