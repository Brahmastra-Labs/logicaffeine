//! synth_strategy_registry_lock — a block's signoff period holds only under the synthesis strategy it was measured
//! with, so the registry carries both.
//!
//! OpenLane hands abc the clock period as its delay target and the strategy decides what abc does with it: AREA
//! recovers area up to the period, DELAY keeps the logic short. A period found under DELAY 0 is not met by an AREA 0
//! rebuild. `codegen_sva::signoff_blocks` holds each block's strategy beside its period, and `emit_flow_config` builds
//! a registered block under that strategy and nothing else.
//!
//! - A: every registered block's evidence run was synthesized under the registry's strategy (`None` is OpenLane's
//!   default, AREA 0).
//! - B: a Caravel project's macro strategy is its macro's registered strategy.
//! - C: the emitter takes a registered block's strategy from the registry and refuses another.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test synth_strategy_registry_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::caravel_project::CARAVEL_PROJECTS;
use logicaffeine_compile::codegen_sva::flow_config::SynthStrategy;
use logicaffeine_compile::codegen_sva::signoff_blocks::{
    registered_synth_strategy, resolve_synth_strategy, SIGNOFF_BLOCKS,
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// OpenLane's spelling of a registered strategy: `None` is its default.
fn openlane_spelling(s: Option<SynthStrategy>) -> String {
    s.map_or_else(|| "AREA 0".to_string(), |s| s.openlane().to_string())
}

/// The run id a registry entry's basis names.
fn basis_run(basis: &str) -> &str {
    let at = basis.rfind("RUN_").unwrap_or_else(|| panic!("a basis without a run: {basis}"));
    &basis[at..at + basis[at..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).unwrap_or(basis.len() - at)]
}

// ── A. the evidence runs ────────────────────────────────────────────────────────────────────────────

#[test]
fn every_registered_block_was_synthesized_under_its_registered_strategy() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let designs = root().join("tools/OpenLane/designs");
    let mut wrong = Vec::new();
    for b in SIGNOFF_BLOCKS.iter() {
        let run = basis_run(b.basis);
        let configs: Vec<PathBuf> = std::fs::read_dir(&designs)
            .expect("the OpenLane designs")
            .filter_map(|d| d.ok().map(|d| d.path().join("runs").join(run).join("config.tcl")))
            .filter(|p| p.is_file())
            .collect();
        let Some(config) = configs.first() else {
            wrong.push(format!("  {}: no design holds its evidence run {run}", b.name));
            continue;
        };
        let text = std::fs::read_to_string(config).unwrap();
        let built = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("set ::env(SYNTH_STRATEGY)"))
            .map(|v| v.trim().trim_matches('"').trim_matches('{').trim_matches('}').to_string())
            .unwrap_or_else(|| "AREA 0".to_string());
        let want = openlane_spelling(b.synth_strategy);
        if built != want {
            wrong.push(format!("  {}: {run} was synthesized under {built:?}, the registry holds {want:?}", b.name));
        }
    }
    assert!(wrong.is_empty(), "registered strategies off their evidence runs:\n{}", wrong.join("\n"));
}

// ── B. the Caravel macros ───────────────────────────────────────────────────────────────────────────

#[test]
fn every_caravel_macro_is_built_under_its_registered_strategy() {
    for p in CARAVEL_PROJECTS {
        let Some(registered) = registered_synth_strategy(p.macro_module) else {
            continue;
        };
        assert_eq!(
            p.synth_strategy, registered,
            "{}: the Caravel project builds its macro {} under {:?}, the registry holds {:?}",
            p.key, p.macro_module, p.synth_strategy, registered
        );
    }
}

// ── C. the emitter ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_emitter_builds_a_registered_block_under_its_registered_strategy_and_nothing_else() {
    let src = std::fs::read_to_string(root().join("crates/logicaffeine_compile/examples/emit_flow_config.rs"))
        .expect("the emitter");
    assert!(
        src.contains("c.synth_strategy = resolve_synth_strategy(&a[0], asked)"),
        "emit_flow_config must take a block's strategy from resolve_synth_strategy"
    );
    let other = |s: Option<SynthStrategy>| match s {
        Some(SynthStrategy::Delay(0)) => Some(SynthStrategy::Area(0)),
        _ => Some(SynthStrategy::Delay(0)),
    };
    for b in SIGNOFF_BLOCKS.iter() {
        // Asked nothing or the registered one: the registered one. Asked another: refused, naming both.
        assert_eq!(resolve_synth_strategy(b.name, None), Ok(b.synth_strategy), "{}", b.name);
        assert_eq!(resolve_synth_strategy(b.name, Some(b.synth_strategy)), Ok(b.synth_strategy), "{}", b.name);
        let refused = resolve_synth_strategy(b.name, Some(other(b.synth_strategy)))
            .expect_err(&format!("{}: another strategy must be refused", b.name));
        assert!(refused.contains("is not the registered synthesis strategy for") && refused.contains(b.name), "{refused}");
    }
    // An unregistered block takes what was asked.
    for asked in [None, Some(None), Some(Some(SynthStrategy::Delay(2))), Some(Some(SynthStrategy::Area(3)))] {
        assert_eq!(resolve_synth_strategy("no_such_block", asked), Ok(asked.flatten()));
    }
    // Every registry entry's strategy is one OpenLane defines, and the registry answers for every entry.
    for b in SIGNOFF_BLOCKS.iter() {
        assert_eq!(registered_synth_strategy(b.name), Some(b.synth_strategy), "{}", b.name);
        if let Some(s) = b.synth_strategy {
            assert_eq!(SynthStrategy::parse(&s.flag()), Some(s), "{}: {s:?} round-trips its flag", b.name);
        }
    }
    assert_eq!(registered_synth_strategy("no_such_block"), None);
}
