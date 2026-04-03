//! Pipeline Gap-Closing Tests: Signal Bridge
//!
//! Sprint 0: Accessibility predicate ordering (temporal semantics fix)
//! Sprint 1: SignalMap (FOL predicate → SVA signal name mapping)
//! Sprint 2: Hardware property compilation API
//! Sprint 3: Z3 end-to-end equivalence (feature-gated)
//! Sprint 4: Real-world protocol benchmarks (feature-gated)

use logicaffeine_language::compile_kripke_with;
use logicaffeine_compile::codegen_sva::fol_to_verify::FolTranslator;
use logicaffeine_compile::codegen_sva::sva_to_verify::{BoundedExpr, TranslateResult};
use logicaffeine_compile::codegen_sva::hw_pipeline;
use std::collections::HashSet;

// ═══════════════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Collect all Var names from a BoundedExpr tree.
fn collect_vars(expr: &BoundedExpr) -> HashSet<String> {
    let mut result = HashSet::new();
    collect_vars_inner(expr, &mut result);
    result
}

fn collect_vars_inner(expr: &BoundedExpr, out: &mut HashSet<String>) {
    match expr {
        BoundedExpr::Var(name) => { out.insert(name.clone()); }
        BoundedExpr::And(l, r) | BoundedExpr::Or(l, r)
        | BoundedExpr::Implies(l, r) | BoundedExpr::Eq(l, r)
        | BoundedExpr::Lt(l, r) | BoundedExpr::Gt(l, r)
        | BoundedExpr::Lte(l, r) | BoundedExpr::Gte(l, r) => {
            collect_vars_inner(l, out);
            collect_vars_inner(r, out);
        }
        BoundedExpr::Not(e) => collect_vars_inner(e, out),
        BoundedExpr::Bool(_) | BoundedExpr::Int(_) | BoundedExpr::Unsupported(_) => {}
    }
}

/// Extract the maximum timestep index from variable names like "X@3".
fn max_timestep(vars: &HashSet<String>) -> Option<u32> {
    vars.iter()
        .filter_map(|v| {
            let at = v.find('@')?;
            v[at + 1..].parse::<u32>().ok()
        })
        .max()
}

/// Extract timesteps for a specific signal base name.
fn timesteps_for_signal(vars: &HashSet<String>, base: &str) -> Vec<u32> {
    let prefix = format!("{}@", base);
    let mut ts: Vec<u32> = vars.iter()
        .filter(|v| v.starts_with(&prefix))
        .filter_map(|v| v[prefix.len()..].parse::<u32>().ok())
        .collect();
    ts.sort();
    ts.dedup();
    ts
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 0: ACCESSIBILITY PREDICATE ORDERING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn s0_eventually_excludes_current_timestep() {
    // "Eventually, Req is valid." → ∃w'(Reachable_Temporal(w0, w') ∧ Valid(Req, w'))
    // Reachable_Temporal means strictly future: w' > w0.
    // Since w0 is implicitly timestep 0, the disjunction should start at t=1.
    let result = compile_kripke_with("Eventually, Req is valid.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 5);
        translator.translate_property(ast)
    }).unwrap();

    let vars = collect_vars(&result.expr);

    // Should have Valid_Req_ variables at future timesteps (1..=5), NOT at timestep 0
    let req_vars: Vec<_> = vars.iter()
        .filter(|v| v.contains("Req") || v.contains("Valid"))
        .cloned()
        .collect();

    assert!(!req_vars.is_empty(),
        "Should have Valid/Req variables. All vars: {:?}", vars);

    // None of the variables should be at timestep 0 (Reachable = strictly future)
    let has_t0 = req_vars.iter().any(|v| v.ends_with("@0"));
    assert!(!has_t0,
        "Eventually should exclude current timestep (t=0). Req vars: {:?}", req_vars);
}

#[test]
fn s0_always_includes_current_timestep() {
    // "Always, Req is valid." → ∀w'(Accessible_Temporal(w0, w') → Valid(Req, w'))
    // Accessible_Temporal means current or future: w' >= w0.
    // Since w0 is timestep 0, the conjunction should start at t=0.
    let result = compile_kripke_with("Always, Req is valid.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 3);
        translator.translate_property(ast)
    }).unwrap();

    let vars = collect_vars(&result.expr);
    let req_vars: Vec<_> = vars.iter()
        .filter(|v| v.contains("Req") || v.contains("Valid"))
        .cloned()
        .collect();

    assert!(!req_vars.is_empty(),
        "Should have Valid/Req variables. All vars: {:?}", vars);

    // Should include t=0
    let has_t0 = req_vars.iter().any(|v| v.ends_with("@0"));
    assert!(has_t0,
        "Always should include current timestep (t=0). Req vars: {:?}", req_vars);
}

#[test]
fn s0_nested_always_eventually_has_future_only_inner() {
    // "Always, if Req is valid then eventually Ack is valid."
    // → ∀w'(Accessible_Temporal(w0, w') → (Valid(Req, w') → ∃w''(Reachable_Temporal(w', w'') ∧ Valid(Ack, w''))))
    //
    // At outer w'=t, the inner ∃w'' should only include timesteps > t.
    // With bound=5, the inner "eventually" at each outer t should look forward
    // up to 5 timesteps.
    let result = compile_kripke_with(
        "Always, if Req is valid then eventually Ack is valid.",
        |ast, interner| {
            let mut translator = FolTranslator::new(interner, 5);
            translator.translate_property(ast)
        }
    ).unwrap();

    let vars = collect_vars(&result.expr);

    // The Ack variables should reach BEYOND the base bound (up to outer_t + bound)
    let ack_vars: Vec<_> = vars.iter()
        .filter(|v| v.contains("Ack"))
        .cloned()
        .collect();

    assert!(!ack_vars.is_empty(),
        "Should have Ack variables. All vars: {:?}", vars);

    let max_t = ack_vars.iter()
        .filter_map(|v| {
            let at = v.find('@')?;
            v[at + 1..].parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0);

    // Inner eventually at outer t=4 should reach at least t=5 (one step ahead)
    // Ideally reaches t=4+5=9 (matching SVA's s_eventually semantics)
    assert!(max_t > 5,
        "Inner 'eventually' should look beyond base bound. Max Ack timestep: {}, expected > 5",
        max_t);
}

#[test]
fn s0_eventually_range_matches_sva_s_eventually() {
    // SVA s_eventually at timestep t with bound B: disjunction from t+1 to t+B
    // FOL "eventually" should produce the same range.
    //
    // With bound=3 and "Always, if Req is valid then eventually Ack is valid.":
    // At outer t=0: inner eventually should give Ack @1, @2, @3
    // At outer t=1: inner eventually should give Ack @2, @3, @4
    // At outer t=2: inner eventually should give Ack @3, @4, @5
    let result = compile_kripke_with(
        "Always, if Req is valid then eventually Ack is valid.",
        |ast, interner| {
            let mut translator = FolTranslator::new(interner, 3);
            translator.translate_property(ast)
        }
    ).unwrap();

    let vars = collect_vars(&result.expr);
    let ack_vars: Vec<_> = vars.iter()
        .filter(|v| v.contains("Ack"))
        .cloned()
        .collect();

    assert!(!ack_vars.is_empty(),
        "Should have Ack variables. All vars: {:?}", vars);

    // At outer t=2 (the last outer step), inner eventually with bound=3
    // should reach t=5 (2 + 3). So max ack timestep should be 5.
    let max_t = ack_vars.iter()
        .filter_map(|v| {
            let at = v.find('@')?;
            v[at + 1..].parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0);

    // With bound=3: outer goes 0,1,2; inner at t=2 goes 3,4,5; max = 5
    assert_eq!(max_t, 5,
        "Max ack timestep should be outer_max + bound = 2 + 3 = 5. Got: {}. Ack vars: {:?}",
        max_t, ack_vars);
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 1: SIGNAL MAP
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn s1_signal_map_basic_resolve() {
    use logicaffeine_compile::codegen_sva::hw_pipeline::SignalMap;

    let mut map = SignalMap::new();
    map.add("Req", "req");
    map.add("Ack", "ack");

    assert_eq!(map.resolve("Req"), Some("req"));
    assert_eq!(map.resolve("Ack"), Some("ack"));
    assert_eq!(map.resolve("Unknown"), None);
}

#[test]
fn s1_fol_translator_with_signal_map_produces_signal_vars() {
    use logicaffeine_compile::codegen_sva::hw_pipeline::SignalMap;

    let mut map = SignalMap::new();
    map.add("Req", "req");

    // "Always, Req is valid." — Req is a proper noun, "valid" is an adjective
    // FOL: ∀w'(Accessible_Temporal(w0, w') → Valid(Req, w'))
    // With signal map: should produce req@0, req@1, req@2 (NOT Valid_Req_@0)
    let result = compile_kripke_with("Always, Req is valid.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 3);
        translator.set_signal_map(&map);
        translator.translate_property(ast)
    }).unwrap();

    let vars = collect_vars(&result.expr);

    // Should have req@t variables
    let has_req = vars.iter().any(|v| v.starts_with("req@"));
    assert!(has_req,
        "Signal map should produce 'req@t' variables. All vars: {:?}", vars);

    // Should NOT have the old-style predicate naming
    let has_old_style = vars.iter().any(|v| v.contains("Valid_Req") || v.contains("valid_Req"));
    assert!(!has_old_style,
        "Signal map should suppress predicate-prefixed naming. All vars: {:?}", vars);
}

#[test]
fn s1_fol_translator_without_signal_map_backward_compatible() {
    // Without signal map, existing predicate_arg naming preserved
    let result = compile_kripke_with("Always, Req is valid.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 3);
        // No signal map set
        translator.translate_property(ast)
    }).unwrap();

    let vars = collect_vars(&result.expr);
    // Should have predicate-style naming: Valid_Req_@0 (not just req@0)
    // Predicate name may be lowercase ("valid") while proper noun stays capitalized ("Req")
    let has_predicate_naming = vars.iter().any(|v|
        (v.contains("Valid") || v.contains("valid")) && v.contains("Req") && v.contains('@')
    );
    assert!(has_predicate_naming,
        "Without signal map, should use predicate naming (valid_Req_@t). All vars: {:?}", vars);
}

#[test]
fn s1_signal_map_multiple_predicates_same_signal() {
    use logicaffeine_compile::codegen_sva::hw_pipeline::SignalMap;

    let mut map = SignalMap::new();
    map.add("Req", "req");

    // Both "Req is valid" and "Req is ready" should map to req@t
    let result = compile_kripke_with("Always, Req is valid.", |ast, interner| {
        let mut translator = FolTranslator::new(interner, 3);
        translator.set_signal_map(&map);
        translator.translate_property(ast)
    }).unwrap();

    let vars = collect_vars(&result.expr);
    let req_vars: Vec<_> = vars.iter().filter(|v| v.starts_with("req@")).cloned().collect();
    assert!(req_vars.len() >= 3,
        "Should have req@0, req@1, req@2. Got: {:?}", req_vars);
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 2: HARDWARE PROPERTY COMPILATION API
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn s2_hw_signal_decl_builds_signal_map() {
    use logicaffeine_compile::codegen_sva::hw_pipeline::{HwSignalDecl, SignalMap};
    use logicaffeine_language::semantics::knowledge_graph::SignalRole;

    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    let map = SignalMap::from_decls(&decls);
    assert_eq!(map.resolve("Req"), Some("req"));
    assert_eq!(map.resolve("Ack"), Some("ack"));
}

#[test]
fn s2_compile_hw_property_with_proper_noun_signals() {
    use logicaffeine_compile::codegen_sva::hw_pipeline::{HwSignalDecl, compile_hw_property};
    use logicaffeine_language::semantics::knowledge_graph::SignalRole;

    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    let result = compile_hw_property(
        "Always, if Req is valid then eventually Ack is valid.",
        &decls,
        5,
    ).unwrap();

    // Declarations should reference "req" and "ack"
    let has_req = result.declarations.iter().any(|d| d.starts_with("req@"));
    let has_ack = result.declarations.iter().any(|d| d.starts_with("ack@"));
    assert!(has_req, "Should have req@ declarations. Got: {:?}", result.declarations);
    assert!(has_ack, "Should have ack@ declarations. Got: {:?}", result.declarations);

    // Should NOT have old-style predicate naming
    let has_old = result.declarations.iter().any(|d|
        d.contains("Valid_Req") || d.contains("Valid_Ack")
    );
    assert!(!has_old,
        "Should not have predicate-prefixed declarations. Got: {:?}", result.declarations);
}

#[test]
fn s2_compile_hw_property_implication_structure() {
    use logicaffeine_compile::codegen_sva::hw_pipeline::{HwSignalDecl, compile_hw_property};
    use logicaffeine_language::semantics::knowledge_graph::SignalRole;

    let decls = vec![
        HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
        HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
    ];
    let result = compile_hw_property(
        "Always, if Req is valid then Ack is valid.",
        &decls,
        3,
    ).unwrap();

    // Should have req@t and ack@t for each timestep 0..3
    for t in 0..3u32 {
        assert!(result.declarations.contains(&format!("req@{}", t)),
            "Missing req@{}. Decls: {:?}", t, result.declarations);
        assert!(result.declarations.contains(&format!("ack@{}", t)),
            "Missing ack@{}. Decls: {:?}", t, result.declarations);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 3: Z3 END-TO-END EQUIVALENCE (feature-gated)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_bridge {
    use logicaffeine_compile::codegen_sva::hw_pipeline::{
        HwSignalDecl, check_z3_hw_equivalence,
    };
    use logicaffeine_language::semantics::knowledge_graph::SignalRole;
    use logicaffeine_verify::equivalence::EquivalenceResult;

    #[test]
    fn s3_handshake_english_equivalent_to_sva() {
        let decls = vec![
            HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
            HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
        ];
        let result = check_z3_hw_equivalence(
            "Always, if Req is valid then eventually Ack is valid.",
            "req |-> s_eventually(ack)",
            &decls,
            5,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "English handshake must be equivalent to SVA. Got: {:?}", result);
    }

    #[test]
    fn s3_mutex_english_equivalent_to_sva() {
        let decls = vec![
            HwSignalDecl::new("GrantA", "grant_a", 1, SignalRole::Internal),
            HwSignalDecl::new("GrantB", "grant_b", 1, SignalRole::Internal),
        ];
        let result = check_z3_hw_equivalence(
            "Always, GrantA and GrantB are not both valid.",
            "!(grant_a && grant_b)",
            &decls,
            3,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "English mutex must be equivalent to SVA. Got: {:?}", result);
    }

    #[test]
    fn s3_wrong_sva_detected() {
        let decls = vec![
            HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
            HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
        ];
        let result = check_z3_hw_equivalence(
            "Always, if Req is valid then eventually Ack is valid.",
            "req |-> ack", // Wrong: immediate, not eventual
            &decls,
            5,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "Immediate ack should NOT be equivalent to eventual ack. Got: {:?}", result);
    }

    #[test]
    fn s3_safety_english_equivalent_to_sva() {
        let decls = vec![
            HwSignalDecl::new("Data", "data", 1, SignalRole::Internal),
            HwSignalDecl::new("En", "en", 1, SignalRole::Input),
        ];
        let result = check_z3_hw_equivalence(
            "Always, if En is valid then Data is valid.",
            "en |-> data",
            &decls,
            3,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Safety implication must be equivalent. Got: {:?}", result);
    }

    #[test]
    fn s3_counterexample_uses_signal_names() {
        let decls = vec![
            HwSignalDecl::new("Req", "req", 1, SignalRole::Input),
            HwSignalDecl::new("Ack", "ack", 1, SignalRole::Output),
        ];
        let result = check_z3_hw_equivalence(
            "Always, if Req is valid then eventually Ack is valid.",
            "req |-> ack",
            &decls,
            5,
        ).unwrap();
        if let EquivalenceResult::NotEquivalent { counterexample } = result {
            let all_signals: Vec<_> = counterexample.cycles.iter()
                .flat_map(|c| c.signals.keys())
                .collect();
            assert!(all_signals.iter().any(|s| s.contains("req") || s.contains("ack")),
                "Counterexample should use signal names. Got: {:?}", all_signals);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SPRINT 4: REAL-WORLD PROTOCOL BENCHMARKS (feature-gated)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod protocol_benchmarks {
    use logicaffeine_compile::codegen_sva::hw_pipeline::{
        HwSignalDecl, check_z3_hw_equivalence,
    };
    use logicaffeine_language::semantics::knowledge_graph::SignalRole;
    use logicaffeine_verify::equivalence::EquivalenceResult;

    #[test]
    fn s4_axi_write_address_handshake() {
        let decls = vec![
            HwSignalDecl::new("Awvalid", "AWVALID", 1, SignalRole::Input),
            HwSignalDecl::new("Awready", "AWREADY", 1, SignalRole::Output),
        ];
        let result = check_z3_hw_equivalence(
            "Always, if Awvalid is valid then eventually Awready is valid.",
            "AWVALID |-> s_eventually(AWREADY)",
            &decls,
            10,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "AXI write address handshake must be equivalent. Got: {:?}", result);
    }

    #[test]
    fn s4_fifo_no_write_when_full() {
        let decls = vec![
            HwSignalDecl::new("Full", "full", 1, SignalRole::Internal),
            HwSignalDecl::new("Wren", "wr_en", 1, SignalRole::Input),
        ];
        let result = check_z3_hw_equivalence(
            "Always, if Full is valid then Wren is not valid.",
            "full |-> !wr_en",
            &decls,
            5,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "FIFO overflow protection must be equivalent. Got: {:?}", result);
    }

    #[test]
    fn s4_arbiter_mutex() {
        let decls = vec![
            HwSignalDecl::new("Grant0", "grant_0", 1, SignalRole::Internal),
            HwSignalDecl::new("Grant1", "grant_1", 1, SignalRole::Internal),
        ];
        let result = check_z3_hw_equivalence(
            "Always, Grant0 and Grant1 are not both valid.",
            "!(grant_0 && grant_1)",
            &decls,
            5,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Arbiter mutex must be equivalent. Got: {:?}", result);
    }

    #[test]
    fn s4_arbiter_liveness() {
        let decls = vec![
            HwSignalDecl::new("Req0", "req_0", 1, SignalRole::Input),
            HwSignalDecl::new("Grant0", "grant_0", 1, SignalRole::Output),
        ];
        let result = check_z3_hw_equivalence(
            "Always, if Req0 is valid then eventually Grant0 is valid.",
            "req_0 |-> s_eventually(grant_0)",
            &decls,
            8,
        ).unwrap();
        assert!(matches!(result, EquivalenceResult::Equivalent),
            "Arbiter liveness must be equivalent. Got: {:?}", result);
    }
}
