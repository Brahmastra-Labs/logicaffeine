//! SUPERCRUSH Sprint S2A: Multi-Clock Domain Modeling

#![cfg(feature = "verification")]

use logicaffeine_verify::multiclock::{verify_multiclock, MultiClockModel, MultiClockResult, ClockDomain};
use logicaffeine_verify::VerifyExpr;
use std::collections::HashMap;

#[test]
fn multiclock_single_domain_fallback() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::iff(
        VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"),
    ));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: Some(100_000_000), ratio: None }],
        init: VerifyExpr::var("p@0"),
        transitions,
        property: VerifyExpr::var("p@t"),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "Single domain should fall back to standard. Got: {:?}", result);
}

#[test]
fn multiclock_two_domains() {
    let mut transitions = HashMap::new();
    transitions.insert("fast".into(), VerifyExpr::var("f@t1"));
    transitions.insert("slow".into(), VerifyExpr::var("s@t1"));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "fast".into(), frequency: Some(200_000_000), ratio: Some((2, 1)) },
            ClockDomain { name: "slow".into(), frequency: Some(100_000_000), ratio: Some((1, 1)) },
        ],
        init: VerifyExpr::and(VerifyExpr::var("f@0"), VerifyExpr::var("s@0")),
        transitions,
        property: VerifyExpr::var("f@t"),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "Two-domain safe property. Got: {:?}", result);
}

#[test]
fn multiclock_unsafe() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::not(VerifyExpr::var("ok@t1")));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: None, ratio: None }],
        init: VerifyExpr::var("ok@0"),
        transitions,
        property: VerifyExpr::var("ok@t"),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Unsafe { .. }),
        "Transition to false should be unsafe. Got: {:?}", result);
}

#[test]
fn multiclock_no_domains_safe() {
    let model = MultiClockModel {
        domains: vec![],
        init: VerifyExpr::bool(true),
        transitions: HashMap::new(),
        property: VerifyExpr::bool(true),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "Empty model should be safe. Got: {:?}", result);
}

#[test]
fn multiclock_three_domains() {
    let mut transitions = HashMap::new();
    transitions.insert("a".into(), VerifyExpr::var("x@t1"));
    transitions.insert("b".into(), VerifyExpr::var("y@t1"));
    transitions.insert("c".into(), VerifyExpr::var("z@t1"));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "a".into(), frequency: None, ratio: None },
            ClockDomain { name: "b".into(), frequency: None, ratio: None },
            ClockDomain { name: "c".into(), frequency: None, ratio: None },
        ],
        init: VerifyExpr::and(VerifyExpr::var("x@0"), VerifyExpr::and(VerifyExpr::var("y@0"), VerifyExpr::var("z@0"))),
        transitions,
        property: VerifyExpr::var("x@t"),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "Three-domain safe. Got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// EXTENDED MULTI-CLOCK TESTS — SPEC DELTA (15 additional)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn multiclock_ratio_2_1() {
    let mut transitions = HashMap::new();
    transitions.insert("fast".into(), VerifyExpr::var("f@t1"));
    transitions.insert("slow".into(), VerifyExpr::var("s@t1"));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "fast".into(), frequency: Some(200_000_000), ratio: Some((2, 1)) },
            ClockDomain { name: "slow".into(), frequency: Some(100_000_000), ratio: Some((1, 1)) },
        ],
        init: VerifyExpr::and(VerifyExpr::var("f@0"), VerifyExpr::var("s@0")),
        transitions,
        property: VerifyExpr::and(VerifyExpr::var("f@t"), VerifyExpr::var("s@t")),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "2:1 ratio should handle correctly. Got: {:?}", result);
}

#[test]
fn multiclock_independent_unroll() {
    let mut transitions = HashMap::new();
    transitions.insert("clk1".into(), VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")));
    transitions.insert("clk2".into(), VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "clk1".into(), frequency: None, ratio: None },
            ClockDomain { name: "clk2".into(), frequency: None, ratio: None },
        ],
        init: VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0")),
        transitions,
        property: VerifyExpr::and(VerifyExpr::var("a@t"), VerifyExpr::var("b@t")),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "Independent domain unroll should be safe. Got: {:?}", result);
}

#[test]
fn multiclock_cross_domain_property() {
    // Property spans both domains
    let mut transitions = HashMap::new();
    transitions.insert("d1".into(), VerifyExpr::var("x@t1"));
    transitions.insert("d2".into(), VerifyExpr::var("y@t1"));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "d1".into(), frequency: None, ratio: None },
            ClockDomain { name: "d2".into(), frequency: None, ratio: None },
        ],
        init: VerifyExpr::and(VerifyExpr::var("x@0"), VerifyExpr::var("y@0")),
        transitions,
        property: VerifyExpr::or(VerifyExpr::var("x@t"), VerifyExpr::var("y@t")),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "Cross-domain OR property. Got: {:?}", result);
}

#[test]
fn multiclock_deterministic() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::var("p@t1"));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: None, ratio: None }],
        init: VerifyExpr::var("p@0"),
        transitions,
        property: VerifyExpr::var("p@t"),
    };
    let r1 = verify_multiclock(&model, 3);
    let model2 = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: None, ratio: None }],
        init: VerifyExpr::var("p@0"),
        transitions: {
            let mut t = HashMap::new();
            t.insert("clk".into(), VerifyExpr::var("p@t1"));
            t
        },
        property: VerifyExpr::var("p@t"),
    };
    let r2 = verify_multiclock(&model2, 3);
    let both = matches!(r1, MultiClockResult::Safe) && matches!(r2, MultiClockResult::Safe);
    assert!(both, "Same model → same result");
}

#[test]
fn multiclock_frequency_info_preserved() {
    // Just verify frequency info doesn't cause issues
    let mut transitions = HashMap::new();
    transitions.insert("fast_clk".into(), VerifyExpr::var("f@t1"));
    let model = MultiClockModel {
        domains: vec![ClockDomain {
            name: "fast_clk".into(),
            frequency: Some(500_000_000), // 500 MHz
            ratio: Some((5, 1)),
        }],
        init: VerifyExpr::var("f@0"),
        transitions,
        property: VerifyExpr::var("f@t"),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "High frequency should not affect correctness. Got: {:?}", result);
}

#[test]
fn multiclock_implication_property() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    ));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: None, ratio: None }],
        init: VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0")),
        transitions,
        property: VerifyExpr::implies(VerifyExpr::var("a@t"), VerifyExpr::var("b@t")),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "a→b with both true. Got: {:?}", result);
}

#[test]
fn multiclock_four_domains() {
    let mut transitions = HashMap::new();
    for name in &["d1", "d2", "d3", "d4"] {
        let sig = format!("{}@t1", name);
        transitions.insert(name.to_string(), VerifyExpr::var(&sig));
    }
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "d1".into(), frequency: None, ratio: None },
            ClockDomain { name: "d2".into(), frequency: None, ratio: None },
            ClockDomain { name: "d3".into(), frequency: None, ratio: None },
            ClockDomain { name: "d4".into(), frequency: None, ratio: None },
        ],
        init: VerifyExpr::and(
            VerifyExpr::and(VerifyExpr::var("d1@0"), VerifyExpr::var("d2@0")),
            VerifyExpr::and(VerifyExpr::var("d3@0"), VerifyExpr::var("d4@0")),
        ),
        transitions,
        property: VerifyExpr::var("d1@t"),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "Four-domain safe. Got: {:?}", result);
}

#[test]
fn multiclock_unsafe_cross_domain() {
    // One domain's signal goes false, property checks it
    let mut transitions = HashMap::new();
    transitions.insert("good".into(), VerifyExpr::var("ok@t1"));
    transitions.insert("bad".into(), VerifyExpr::not(VerifyExpr::var("fail@t1")));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "good".into(), frequency: None, ratio: None },
            ClockDomain { name: "bad".into(), frequency: None, ratio: None },
        ],
        init: VerifyExpr::and(VerifyExpr::var("ok@0"), VerifyExpr::var("fail@0")),
        transitions,
        property: VerifyExpr::var("fail@t"),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Unsafe { .. }),
        "Bad domain transition to false should be unsafe. Got: {:?}", result);
}

#[test]
fn multiclock_trivially_true_property() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::bool(true));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: None, ratio: None }],
        init: VerifyExpr::bool(true),
        transitions,
        property: VerifyExpr::bool(true),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "True property should be safe. Got: {:?}", result);
}

#[test]
fn multiclock_negation_property() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t")));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: None, ratio: None }],
        init: VerifyExpr::not(VerifyExpr::var("p@0")),
        transitions,
        property: VerifyExpr::not(VerifyExpr::var("p@t")),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "NOT p with p false. Got: {:?}", result);
}

#[test]
fn multiclock_conjunction_property_multi_domain() {
    let mut transitions = HashMap::new();
    transitions.insert("fast".into(), VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")));
    transitions.insert("slow".into(), VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "fast".into(), frequency: None, ratio: Some((2, 1)) },
            ClockDomain { name: "slow".into(), frequency: None, ratio: Some((1, 1)) },
        ],
        init: VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0")),
        transitions,
        property: VerifyExpr::and(VerifyExpr::var("a@t"), VerifyExpr::var("b@t")),
    };
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "Conjunction across domains should be safe. Got: {:?}", result);
}

#[test]
fn multiclock_gated_clock_simulation() {
    // Simulate gated clock: domain only active when enable is true
    let mut transitions = HashMap::new();
    transitions.insert("gated".into(), VerifyExpr::var("p@t1"));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "gated".into(), frequency: None, ratio: None }],
        init: VerifyExpr::var("p@0"),
        transitions,
        property: VerifyExpr::var("p@t"),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "Gated clock should be safe. Got: {:?}", result);
}

#[test]
fn multiclock_regression_boolean_unchanged() {
    // Boolean-only single domain — same as CRUSH behavior
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    ));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: Some(100_000_000), ratio: None }],
        init: VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0")),
        transitions,
        property: VerifyExpr::and(VerifyExpr::var("a@t"), VerifyExpr::var("b@t")),
    };
    let result = verify_multiclock(&model, 3);
    assert!(matches!(result, MultiClockResult::Safe),
        "Boolean regression should be safe. Got: {:?}", result);
}

#[test]
fn multiclock_large_bound() {
    let mut transitions = HashMap::new();
    transitions.insert("clk".into(), VerifyExpr::var("p@t1"));
    let model = MultiClockModel {
        domains: vec![ClockDomain { name: "clk".into(), frequency: None, ratio: None }],
        init: VerifyExpr::var("p@0"),
        transitions,
        property: VerifyExpr::var("p@t"),
    };
    let result = verify_multiclock(&model, 20);
    assert!(matches!(result, MultiClockResult::Safe),
        "Large bound should still work. Got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// PER-DOMAIN UNROLLING AND RATIO TESTS — expose fake conjoined impl
// ═══════════════════════════════════════════════════════════════════════════

use logicaffeine_verify::multiclock::compute_schedule;

#[test]
fn multiclock_ratio_used_in_verification() {
    // Domain "fast" has ratio (2,1) meaning it fires twice per period.
    // Domain "slow" has ratio (1,1) meaning it fires once per period.
    // The schedule within one LCM period (2 global steps) should be:
    //   step 0: fast fires, slow fires
    //   step 1: fast fires, slow does NOT fire (frame condition)
    //
    // With both domains toggling, after 2 global steps the slow signal
    // should have toggled only once (not twice). The current AND-all
    // impl toggles slow every step.
    let schedule = compute_schedule(&[
        ClockDomain { name: "fast".into(), frequency: Some(200_000_000), ratio: Some((2, 1)) },
        ClockDomain { name: "slow".into(), frequency: Some(100_000_000), ratio: Some((1, 1)) },
    ]);
    // schedule[global_step] -> Vec<bool> indicating which domains fire
    // Over one LCM period = 2 global ticks:
    assert_eq!(schedule.len(), 2, "LCM period for (2,1):(1,1) should be 2 global steps");
    // fast fires at both steps
    assert!(schedule[0][0], "fast should fire at step 0");
    assert!(schedule[1][0], "fast should fire at step 1");
    // slow fires only at step 0
    assert!(schedule[0][1], "slow should fire at step 0");
    assert!(!schedule[1][1], "slow should NOT fire at step 1 (ratio 1:1 vs 2:1)");
}

#[test]
fn multiclock_per_domain_unroll() {
    // Each domain unrolls independently at its own rate.
    // fast (3,1) fires 3 times per period. slow (1,1) fires 1 time per period.
    // LCM period = 3 global steps.
    let schedule = compute_schedule(&[
        ClockDomain { name: "fast".into(), frequency: None, ratio: Some((3, 1)) },
        ClockDomain { name: "slow".into(), frequency: None, ratio: Some((1, 1)) },
    ]);
    assert_eq!(schedule.len(), 3, "LCM period for (3,1):(1,1) should be 3 global steps");
    let fast_fires: usize = schedule.iter().filter(|s| s[0]).count();
    let slow_fires: usize = schedule.iter().filter(|s| s[1]).count();
    assert_eq!(fast_fires, 3, "fast should fire 3 times per period");
    assert_eq!(slow_fires, 1, "slow should fire 1 time per period");
}

#[test]
fn multiclock_cross_domain_timing() {
    // Property depends on relative timing between domains.
    //
    // fast domain (2,1): f toggles every fast tick: f@t1 <=> NOT f@t
    // slow domain (1,1): s latches current f value: s@t1 <=> f@t
    //
    // Init: f=true, s=true
    // Property: s@t (slow signal always true)
    //
    // With correct interleaving (slow fires once per 2 global steps):
    //   step 0->1: fast fires (f=F), slow fires (s=f@0=T) => f=F, s=T
    //   step 1->2: fast fires (f=T), slow frame (s=T held) => f=T, s=T
    //   step 2->3: fast fires (f=F), slow fires (s=f@2=T) => f=F, s=T
    //   Pattern: s is ALWAYS true => SAFE
    //
    // With naive AND-all (both fire every step):
    //   step 0->1: f=F, s=f@0=T => f=F, s=T
    //   step 1->2: f=T, s=f@1=F => f=T, s=F   <-- s becomes false!
    //   Pattern: s alternates T/F => UNSAFE at step 2
    let mut transitions = HashMap::new();
    transitions.insert("fast".into(), VerifyExpr::iff(
        VerifyExpr::var("f@t1"), VerifyExpr::not(VerifyExpr::var("f@t")),
    ));
    transitions.insert("slow".into(), VerifyExpr::iff(
        VerifyExpr::var("s@t1"), VerifyExpr::var("f@t"),
    ));
    let model = MultiClockModel {
        domains: vec![
            ClockDomain { name: "fast".into(), frequency: None, ratio: Some((2, 1)) },
            ClockDomain { name: "slow".into(), frequency: None, ratio: Some((1, 1)) },
        ],
        init: VerifyExpr::and(VerifyExpr::var("f@0"), VerifyExpr::var("s@0")),
        transitions,
        property: VerifyExpr::var("s@t"),
    };
    // With correct interleaving: s is always true because slow only samples f
    // on even steps when f has just toggled back to true.
    // With naive AND-all: s=false at step 2 because slow samples f=false at step 1.
    let result = verify_multiclock(&model, 5);
    assert!(matches!(result, MultiClockResult::Safe),
        "Slow domain should hold s via frame condition on non-fire steps. Got: {:?}", result);
}
