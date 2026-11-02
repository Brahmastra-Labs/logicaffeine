use super::*;
use std::collections::HashSet;

#[test]
fn registry_has_one_row_per_opt_in_order() {
    // Every row's opt must equal its index (discriminant == bit == index).
    for (i, m) in REGISTRY.iter().enumerate() {
        assert_eq!(
            m.opt as usize, i,
            "registry row {i} out of order: {:?}",
            m.opt
        );
    }
}

#[test]
fn keywords_are_unique_lowercase_single_tokens() {
    let mut seen = HashSet::new();
    for m in REGISTRY {
        assert!(seen.insert(m.keyword), "duplicate keyword {}", m.keyword);
        assert_eq!(
            m.keyword,
            m.keyword.to_ascii_lowercase(),
            "keyword not lowercase: {}",
            m.keyword
        );
        assert!(!m.keyword.is_empty(), "empty keyword");
        assert!(
            !m.keyword.contains(char::is_whitespace),
            "keyword has whitespace: {}",
            m.keyword
        );
    }
}

#[test]
fn requires_and_conflicts_reference_real_variants_no_cycle() {
    // All references resolve (they are typed `Opt`, so this is structural):
    // assert no opt requires itself and there is no trivial requires-cycle.
    for m in REGISTRY {
        assert!(!m.requires.contains(&m.opt), "{:?} requires itself", m.opt);
        assert!(
            !m.conflicts.contains(&m.opt),
            "{:?} conflicts with itself",
            m.opt
        );
        for &r in m.requires {
            // No 2-cycle: the required opt must not require this one back.
            assert!(
                !r.meta().requires.contains(&m.opt),
                "requires cycle: {:?} <-> {:?}",
                m.opt,
                r
            );
        }
    }
}

#[test]
fn run_path_opt_costs_match_fixture() {
    // The RUN-path opts `optimize_for_run` actually gates, with the cost that
    // decides which hotness tier pays for them (HOTSWAP §3). Pinned as a fixture
    // so a misclassification is a hard failure, never silently absorbed.
    use OptCost::{Cheap, Heavy, Medium};
    let expect = |opt: Opt, c: OptCost| assert_eq!(opt.cost(), c, "{opt:?} cost");
    for (opt, c) in [
        (Opt::Inline, Cheap),
        (Opt::Tco, Cheap),
        (Opt::DeadCode, Cheap),
        (Opt::Specialize, Medium), // PE-light at T2 / PE-full at T3 via the iteration cap
        (Opt::Comptime, Medium),
        (Opt::LoopCse, Medium),
        (Opt::FloatStrength, Medium),
        (Opt::Cse, Medium),
        (Opt::Affine, Medium),
        (Opt::LoopHoist, Medium),
        (Opt::ClosedForm, Medium),
        (Opt::Fuse, Medium),
        (Opt::Oracle, Medium),
        (Opt::ElemType, Medium),
        (Opt::Unroll, Heavy),
        (Opt::Scalarize, Heavy),
        (Opt::Unfold, Heavy),
    ] {
        expect(opt, c);
    }
}

#[test]
fn requires_cost_monotone() {
    // Well-formedness for the tiered optimizer: every opt's `requires` must cost
    // no more than the opt itself, so a tier that admits the dependent always
    // admits its dependencies (HOTSWAP §4.2). A failure is a HARD STOP — never
    // resolve it by lowering a dependency's cost.
    for m in REGISTRY {
        for &r in m.requires {
            assert!(
                r.cost() <= m.opt.cost(),
                "{:?} (cost {:?}) requires {:?} (cost {:?}) — violates cost monotonicity",
                m.opt,
                m.opt.cost(),
                r,
                r.cost()
            );
        }
    }
}

#[test]
fn tier_budget_and_admits_truth_table() {
    use OptCost::{Cheap, Heavy, Medium};
    assert_eq!(Tier::T0.budget(), None);
    assert_eq!(Tier::T1.budget(), Some(Cheap));
    assert_eq!(Tier::T2.budget(), Some(Medium));
    assert_eq!(Tier::T3.budget(), Some(Heavy));

    let on = OptimizationConfig::all_on();
    // T0 optimizes nothing.
    assert!(!admits(&on, Tier::T0, Opt::Inline));
    // T1: Cheap only.
    assert!(admits(&on, Tier::T1, Opt::Inline)); // Cheap
    assert!(!admits(&on, Tier::T1, Opt::Cse)); // Medium
    assert!(!admits(&on, Tier::T1, Opt::Unroll)); // Heavy
                                                  // T2: Cheap + Medium.
    assert!(admits(&on, Tier::T2, Opt::Inline));
    assert!(admits(&on, Tier::T2, Opt::Cse));
    assert!(!admits(&on, Tier::T2, Opt::Unroll));
    // T3: everything.
    assert!(admits(&on, Tier::T3, Opt::Inline));
    assert!(admits(&on, Tier::T3, Opt::Cse));
    assert!(admits(&on, Tier::T3, Opt::Unroll));
    // admits still respects cfg.is_on — a disabled opt never runs at any tier.
    let off = OptimizationConfig::all_on().disable(Opt::Inline);
    assert!(!admits(&off, Tier::T3, Opt::Inline));
    // Tiers order like their budgets.
    assert!(Tier::T0 < Tier::T1 && Tier::T1 < Tier::T2 && Tier::T2 < Tier::T3);
}

#[test]
fn opt_count_matches_registry() {
    assert_eq!(OPT_COUNT, REGISTRY.len());
}

#[test]
fn tier_thresholds_default_and_tier_for() {
    let t = TierThresholds::default();
    assert_eq!((t.t1, t.t2, t.t3), (8, 32, 100));
    assert_eq!(t.tier_for(0), Tier::T0);
    assert_eq!(t.tier_for(7), Tier::T0);
    assert_eq!(t.tier_for(8), Tier::T1);
    assert_eq!(t.tier_for(31), Tier::T1);
    assert_eq!(t.tier_for(32), Tier::T2);
    assert_eq!(t.tier_for(99), Tier::T2);
    assert_eq!(t.tier_for(100), Tier::T3);
    assert_eq!(t.tier_for(10_000), Tier::T3);
}

#[test]
fn hotswap_mode_effective_tier() {
    let th = TierThresholds::default();
    let mk = |mode, force_tier| HotswapConfig {
        mode,
        thresholds: th,
        force_tier,
        pins: PinSet::none(),
    };
    // Eager: always T3 regardless of count.
    assert_eq!(mk(TierMode::Eager, None).effective_tier(0), Tier::T3);
    // Baseline: always T0.
    assert_eq!(
        mk(TierMode::Baseline, None).effective_tier(10_000),
        Tier::T0
    );
    // Tiered: climbs by hotness.
    let tiered = mk(TierMode::Tiered, None);
    assert_eq!(tiered.effective_tier(0), Tier::T0);
    assert_eq!(tiered.effective_tier(8), Tier::T1);
    assert_eq!(tiered.effective_tier(32), Tier::T2);
    assert_eq!(tiered.effective_tier(100), Tier::T3);
    // force_tier overrides everything (the determinism lever for tests).
    let forced = mk(TierMode::Tiered, Some(Tier::T2));
    assert_eq!(forced.effective_tier(0), Tier::T2);
    assert_eq!(forced.effective_tier(10_000), Tier::T2);
}

#[test]
fn hotswap_from_spec_modes_and_force() {
    let th = TierThresholds::default();
    // LOGOS_HOTSWAP=off forces Eager.
    assert_eq!(
        HotswapConfig::from_spec(Some("off"), Some("baseline"), None, th, None).mode,
        TierMode::Eager
    );
    // Profiles.
    assert_eq!(
        HotswapConfig::from_spec(None, Some("eager"), None, th, None).mode,
        TierMode::Eager
    );
    assert_eq!(
        HotswapConfig::from_spec(None, Some("baseline"), None, th, None).mode,
        TierMode::Baseline
    );
    assert_eq!(
        HotswapConfig::from_spec(None, Some("tiered"), None, th, None).mode,
        TierMode::Tiered
    );
    // Default mode is Eager (the §12.2 ratchet — Tiered is opt-in until proven).
    assert_eq!(
        HotswapConfig::from_spec(None, None, None, th, None).mode,
        TierMode::Eager
    );
    assert_eq!(HotswapConfig::default().mode, TierMode::Eager);
    // force_tier parses number and word forms.
    assert_eq!(
        HotswapConfig::from_spec(None, None, Some("2"), th, None).force_tier,
        Some(Tier::T2)
    );
    assert_eq!(
        HotswapConfig::from_spec(None, None, Some("veryhot"), th, None).force_tier,
        Some(Tier::T3)
    );
    assert_eq!(
        HotswapConfig::from_spec(None, None, Some("nonsense"), th, None).force_tier,
        None
    );
}

#[test]
fn run_tier_maps_mode_to_upfront_tier() {
    let th = TierThresholds::default();
    let mk = |mode, force_tier| HotswapConfig {
        mode,
        thresholds: th,
        force_tier,
        pins: PinSet::none(),
    };
    // Eager pays the full optimizer upfront; Tiered/Baseline start at the baseline.
    assert_eq!(mk(TierMode::Eager, None).run_tier(), Tier::T3);
    assert_eq!(mk(TierMode::Tiered, None).run_tier(), Tier::T0);
    assert_eq!(mk(TierMode::Baseline, None).run_tier(), Tier::T0);
    // force_tier overrides the mode (test determinism).
    assert_eq!(mk(TierMode::Tiered, Some(Tier::T2)).run_tier(), Tier::T2);
    assert_eq!(mk(TierMode::Eager, Some(Tier::T1)).run_tier(), Tier::T1);
}

#[test]
fn admits_pinned_overrides_cost_tier() {
    let on = OptimizationConfig::all_on();
    let mut pins = PinSet::none();
    pins.set(Opt::Unfold, Pin::Never);
    pins.set(Opt::Fuse, Pin::Eager);
    pins.set(Opt::Specialize, Pin::At(Tier::T2));
    let hs = HotswapConfig {
        mode: TierMode::Tiered,
        thresholds: TierThresholds::default(),
        force_tier: None,
        pins,
    };
    // Never: off at every tier.
    assert!(!admits_pinned(&on, &hs, Tier::T3, Opt::Unfold));
    // Eager: on as soon as warm (T1), despite Medium cost.
    assert!(admits_pinned(&on, &hs, Tier::T1, Opt::Fuse));
    // At(T2): off at T1, on at T2.
    assert!(!admits_pinned(&on, &hs, Tier::T1, Opt::Specialize));
    assert!(admits_pinned(&on, &hs, Tier::T2, Opt::Specialize));
    // Unpinned: cost-derived (matches `admits`).
    assert!(admits_pinned(&on, &hs, Tier::T1, Opt::Inline));
    assert!(!admits_pinned(&on, &hs, Tier::T1, Opt::Cse));
    // A disabled opt never runs regardless of pin.
    let off = OptimizationConfig::all_on().disable(Opt::Fuse);
    assert!(!admits_pinned(&off, &hs, Tier::T3, Opt::Fuse));
}

#[test]
fn pin_spec_parses_keyword_value_pairs() {
    let mut pins = PinSet::none();
    apply_pin_spec(
        &mut pins,
        "specialize:eager, fuse:t1 ; unfold:never, bogus:t2, cse:nonsense",
    );
    assert_eq!(pins.get(Opt::Specialize), Some(Pin::Eager));
    assert_eq!(pins.get(Opt::Fuse), Some(Pin::At(Tier::T1)));
    assert_eq!(pins.get(Opt::Unfold), Some(Pin::Never));
    assert_eq!(pins.get(Opt::Cse), None, "unparseable value is ignored");
}

#[test]
fn decorate_source_inserts_file_level_decorators() {
    let src = "## To f (x: Int) -> Int:\n    Return x.\n\n## Main\nShow 1.\n";
    let d = decorate_source(src, &["scalarize", "unroll"]);
    assert!(
        d.contains("## No scalarize\n## No unroll\n## Main"),
        "got:\n{d}"
    );
    assert!(
        d.starts_with("## To f"),
        "functions stay before the decorators"
    );
    // Empty list is a no-op.
    assert_eq!(decorate_source(src, &[]), src);
    // A library with no `## Main` appends the decorators (EOF = file-level).
    let lib = "## To f (x: Int) -> Int:\n    Return x.\n";
    assert!(decorate_source(lib, &["cse"]).ends_with("## No cse\n"));
}

#[test]
fn decorate_tiers_inserts_file_level_pins() {
    let src = "## To f (x: Int) -> Int:\n    Return x.\n\n## Main\nShow 1.\n";
    let d = decorate_tiers(src, &[("specialize", "eager"), ("unfold", "never")]);
    assert!(
        d.contains("## Tier specialize eager\n## Tier unfold never\n## Main"),
        "got:\n{d}"
    );
    assert!(
        d.starts_with("## To f"),
        "functions stay before the decorators"
    );
    assert_eq!(decorate_tiers(src, &[]), src, "empty list is a no-op");
    let lib = "## To f (x: Int) -> Int:\n    Return x.\n";
    assert!(decorate_tiers(lib, &[("fuse", "t1")]).ends_with("## Tier fuse t1\n"));
}

#[test]
fn by_keyword_round_trips() {
    for m in REGISTRY {
        assert_eq!(by_keyword(m.keyword), Some(m.opt));
        assert_eq!(by_keyword(&m.keyword.to_uppercase()), Some(m.opt));
    }
    assert_eq!(by_keyword("nonsense"), None);
}

#[test]
fn all_on_enables_everything_all_off_nothing() {
    let on = OptimizationConfig::all_on();
    let off = OptimizationConfig::all_off();
    for m in REGISTRY {
        assert!(on.is_on(m.opt), "{:?} should be on", m.opt);
        assert!(!off.is_on(m.opt), "{:?} should be off", m.opt);
    }
    assert!(off.is_all_off());
    assert_eq!(OptimizationConfig::default(), on);
}

#[test]
fn serde_round_trips() {
    let cfg = OptimizationConfig::all_on()
        .disable(Opt::Scalarize)
        .disable(Opt::Unroll);
    let json = serde_json::to_string(&cfg).unwrap();
    let back: OptimizationConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(cfg, back);
}

#[test]
fn normalize_disables_dependents_and_reports() {
    // Scalarize requires Cse; turning Cse off must disable Scalarize.
    let mut cfg = OptimizationConfig::all_on().disable(Opt::Cse);
    let report = cfg.normalize();
    assert!(!cfg.is_on(Opt::Scalarize), "Scalarize must follow Cse off");
    assert!(report
        .auto_disabled
        .contains(&(Opt::Scalarize, Reason::DependencyOff(Opt::Cse))));
}

#[test]
fn normalize_oracle_closure() {
    // Oracle off must disable ElemType, OracleHints, Unchecked.
    let mut cfg = OptimizationConfig::all_on().disable(Opt::Oracle);
    cfg.normalize();
    assert!(!cfg.is_on(Opt::ElemType));
    assert!(!cfg.is_on(Opt::OracleHints));
    assert!(!cfg.is_on(Opt::Unchecked));
}

#[test]
fn default_config_is_stable_under_normalize() {
    // The default (all-on) config has NO config-level conflicts: AoS/Interleave
    // is mutually exclusive with Unroll/Scalarize only at RUNTIME (the codegen
    // regime gate), never in the config, so all three stay enabled by default
    // and normalization is a no-op. Guards the regression where a config-level
    // Interleave⇄Unroll/Scalarize conflict silently disabled AoS by default.
    let mut cfg = OptimizationConfig::all_on();
    let report = cfg.normalize();
    assert!(
        report.is_empty(),
        "default config must normalize to a no-op, got {:?}",
        report
    );
    assert!(cfg.is_on(Opt::Interleave));
    assert!(cfg.is_on(Opt::Unroll));
    assert!(cfg.is_on(Opt::Scalarize));
    assert_eq!(cfg, OptimizationConfig::all_on());
}

#[test]
fn safety_profile_drops_all_unsafe() {
    let cfg = Profile::Safety.config();
    for m in REGISTRY {
        if m.emits_unsafe {
            assert!(
                !cfg.is_on(m.opt),
                "Safety must disable unsafe-emitting {:?}",
                m.opt
            );
        }
    }
}

#[test]
fn memory_profile_drops_mem_hogs_keeps_savers() {
    let cfg = Profile::Memory.config();
    assert!(!cfg.is_on(Opt::Unroll));
    assert!(!cfg.is_on(Opt::Scalarize));
    assert!(cfg.is_on(Opt::Narrow) || !OptimizationConfig::all_on().is_on(Opt::Unbox));
    assert!(cfg.is_on(Opt::Unbox), "Memory keeps memory-saving Unbox");
}

#[test]
fn from_toggles_reports_forced_off() {
    let mut t = BTreeMap::new();
    t.insert("cse".to_string(), false);
    let (cfg, forced) = OptimizationConfig::from_toggles(&t);
    assert!(!cfg.is_on(Opt::Cse));
    assert!(!cfg.is_on(Opt::Scalarize));
    assert!(forced.contains(&Opt::Scalarize));
}

#[test]
fn from_spec_master_off_is_all_off() {
    assert!(OptimizationConfig::from_spec(true, None, None).is_all_off());
    assert!(OptimizationConfig::from_spec(true, Some("safety"), Some("cse")).is_all_off());
}

#[test]
fn from_spec_default_is_all_on() {
    assert_eq!(
        OptimizationConfig::from_spec(false, None, None),
        OptimizationConfig::all_on()
    );
}

#[test]
fn from_spec_off_list_disables_listed_keywords() {
    let cfg = OptimizationConfig::from_spec(false, None, Some("scalarize, unroll;nonsense"));
    assert!(!cfg.is_on(Opt::Scalarize));
    assert!(!cfg.is_on(Opt::Unroll));
    assert!(
        cfg.is_on(Opt::Memo),
        "unlisted opts stay on; unknown tokens ignored"
    );
}

#[test]
fn from_spec_off_list_normalizes_dependents() {
    // Disabling Oracle cascades to its dependents.
    let cfg = OptimizationConfig::from_spec(false, None, Some("oracle"));
    assert!(!cfg.is_on(Opt::Oracle));
    assert!(!cfg.is_on(Opt::Unchecked));
    assert!(!cfg.is_on(Opt::OracleHints));
    assert!(!cfg.is_on(Opt::ElemType));
}

#[test]
fn from_spec_profiles_drop_the_right_opts() {
    let safety = OptimizationConfig::from_spec(false, Some("safety"), None);
    for m in REGISTRY {
        if m.emits_unsafe {
            assert!(
                !safety.is_on(m.opt),
                "Safety must drop unsafe-emitting {:?}",
                m.opt
            );
        }
    }
    let memory = OptimizationConfig::from_spec(false, Some("memory"), None);
    assert!(!memory.is_on(Opt::Unroll));
    assert!(memory.is_on(Opt::Unbox), "Memory keeps memory-saving opts");
}

// --- Toggle-linking: the requires graph drives both cascade directions ---

#[test]
fn disabling_any_requirement_cascades_to_dependents() {
    // The toggle-link contract, generic over the registry: for EVERY `requires`
    // edge, turning the requirement off (from all-on) must, after normalize,
    // also turn the dependent off. This is what makes "turn a parent off and
    // its children turn off" hold for every edge, not just the hand-picked ones.
    for m in REGISTRY {
        for &req in m.requires {
            let mut cfg = OptimizationConfig::all_on();
            cfg.set(req, false);
            cfg.normalize();
            assert!(
                !cfg.is_on(m.opt),
                "{:?} requires {:?}; disabling {:?} must cascade {:?} off",
                m.opt,
                req,
                req,
                m.opt
            );
        }
    }
}

#[test]
fn enable_with_requires_pulls_ancestors() {
    // Enabling a leaf from all-off must pull its whole requires-chain on, so the
    // act of turning a child on is not instantly undone by normalize.
    let mut cfg = OptimizationConfig::all_off();
    cfg.enable_with_requires(Opt::Interleave);
    assert!(cfg.is_on(Opt::Interleave));
    assert!(cfg.is_on(Opt::Scalarize), "Interleave needs Scalarize");
    assert!(cfg.is_on(Opt::Cse), "…which needs Cse");
    assert!(!cfg.is_on(Opt::Unroll), "unrelated opts stay off");
    assert!(!cfg.is_on(Opt::Oracle), "unrelated opts stay off");
    let report = cfg.normalize();
    assert!(
        report.is_empty(),
        "an enabled chain must be normalize-stable: {report:?}"
    );
}

// --- relationship_tree: the deterministic per-program chain from one trace ---

fn role_of(tree: &[OptNode], opt: Opt) -> Option<OptRole> {
    tree.iter().find(|n| n.opt == opt).map(|n| n.role)
}
fn depth_of(tree: &[OptNode], opt: Opt) -> usize {
    tree.iter().find(|n| n.opt == opt).unwrap().depth
}
fn pos_of(tree: &[OptNode], opt: Opt) -> usize {
    tree.iter().position(|n| n.opt == opt).unwrap()
}

#[test]
fn relationship_tree_pulls_enabler_parents_for_orphan_children() {
    // coins-like: unchecked/oraclehints/elemtype fire, but their `requires`
    // parent oracle does not — it must still appear, as an Enabler, parent of
    // the children, so the tree is never orphaned.
    let fired = [Opt::Unchecked, Opt::OracleHints, Opt::ElemType];
    let tree = relationship_tree(&fired, &[], &[]);
    assert_eq!(role_of(&tree, Opt::Unchecked), Some(OptRole::Fired));
    assert_eq!(role_of(&tree, Opt::Oracle), Some(OptRole::Enabler));
    assert!(depth_of(&tree, Opt::Oracle) < depth_of(&tree, Opt::Unchecked));
    assert!(
        tree.iter()
            .find(|n| n.opt == Opt::Oracle)
            .unwrap()
            .has_children
    );
}

#[test]
fn relationship_tree_surfaces_preempted_losers() {
    // densemap fired and beat narrowmap: narrowmap must appear as a Preempted
    // node (the "skipped because they don't play nice" opt), annotated with the
    // winner that beat it, even though it never fired.
    let fired = [Opt::DenseMap];
    let preempted = [(Opt::DenseMap, Opt::NarrowMap)];
    let tree = relationship_tree(&fired, &preempted, &[]);
    let node = tree
        .iter()
        .find(|n| n.opt == Opt::NarrowMap)
        .expect("narrowmap present");
    assert_eq!(node.role, OptRole::Preempted);
    assert!(node.preempted_by.contains(&Opt::DenseMap));
    assert_eq!(role_of(&tree, Opt::DenseMap), Some(OptRole::Fired));
}

#[test]
fn relationship_tree_nests_requires_chain_by_depth() {
    // cse → scalarize → interleave all fired: depths 0,1,2, parents first.
    let fired = [Opt::Cse, Opt::Scalarize, Opt::Interleave];
    let tree = relationship_tree(&fired, &[], &[]);
    assert_eq!(depth_of(&tree, Opt::Cse), 0);
    assert_eq!(depth_of(&tree, Opt::Scalarize), 1);
    assert_eq!(depth_of(&tree, Opt::Interleave), 2);
    assert!(pos_of(&tree, Opt::Cse) < pos_of(&tree, Opt::Scalarize));
    assert!(pos_of(&tree, Opt::Scalarize) < pos_of(&tree, Opt::Interleave));
}

#[test]
fn relationship_tree_is_deterministic_and_only_in_play() {
    // The in-play set is exactly fired ∪ losers ∪ requires-closure — nothing
    // else — and the derivation is deterministic (same input → same output).
    let fired = [Opt::DenseMap, Opt::Unchecked];
    let preempted = [(Opt::DenseMap, Opt::NarrowMap)];
    let a = relationship_tree(&fired, &preempted, &[]);
    let b = relationship_tree(&fired, &preempted, &[]);
    let keys = |t: &[OptNode]| {
        t.iter()
            .map(|n| (n.opt, n.depth, n.role))
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&a), keys(&b), "derivation must be deterministic");
    let in_play: std::collections::BTreeSet<Opt> = a.iter().map(|n| n.opt).collect();
    let expected: std::collections::BTreeSet<Opt> =
        [Opt::DenseMap, Opt::NarrowMap, Opt::Unchecked, Opt::Oracle]
            .into_iter()
            .collect();
    assert_eq!(in_play, expected, "only in-play opts appear");
}

#[test]
fn relationship_tree_nests_by_per_program_dependencies() {
    // A per-program dependency (dependent, dep) nests the dependent under the
    // dep even with no static `requires` edge — e.g. dead-code elimination only
    // fired because scalarization produced the dead code. Both fired.
    let fired = [Opt::Scalarize, Opt::DeadCode, Opt::Cse];
    let deps = [(Opt::DeadCode, Opt::Scalarize)];
    let tree = relationship_tree(&fired, &[], &deps);
    // deadcode hangs under scalarize (deeper), and records the dependency.
    assert!(depth_of(&tree, Opt::DeadCode) > depth_of(&tree, Opt::Scalarize));
    assert!(pos_of(&tree, Opt::Scalarize) < pos_of(&tree, Opt::DeadCode));
    let dc = tree.iter().find(|n| n.opt == Opt::DeadCode).unwrap();
    assert!(dc.depends_on.contains(&Opt::Scalarize));
    assert_eq!(dc.role, OptRole::Fired);
    // scalarize has children now (deadcode depends on it).
    assert!(
        tree.iter()
            .find(|n| n.opt == Opt::Scalarize)
            .unwrap()
            .has_children
    );
    // static requires still compose: scalarize under cse.
    assert!(depth_of(&tree, Opt::Scalarize) > depth_of(&tree, Opt::Cse));
}
