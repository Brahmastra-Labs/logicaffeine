//! Byte-lock for `tests/fixtures/studio_example_contract.json` — what every Studio example
//! promises, published for the browser suite.
//!
//! The registry (`ui::examples`) already locks each example's outcome at the engine level;
//! this exports the same promises — the file, the mode it opens in, and its `Expected` — so
//! `scripts/web/test-studio-e2e.mjs` can hold the Studio's own panels to them, one file type at a
//! time. `LOGOS_REGEN=1 cargo nextest run -p logicaffeine-tests --features web-tests --test studio_example_contract`
//! rewrites it from the registry before holding it to the registry.

#![cfg(feature = "web-tests")]

use std::path::PathBuf;

use logicaffeine_web::ui::examples::{
    compiled_runtime_needs, planted_bug_fix, ExampleSpec, Expected, ALL_CODE_EXAMPLES, ALL_HARDWARE_SPECS,
    ALL_LOGIC_EXAMPLES, ALL_MATH_EXAMPLES,
};
use logicaffeine_web::ui::state::StudioMode;

fn contract_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/studio_example_contract.json")
}

/// An example's promise in the browser's terms. The match has no fallback arm: a new kind of
/// promise must say how the Studio shows it before it builds.
fn promise(spec: &ExampleSpec) -> serde_json::Value {
    use serde_json::json;
    let fixed = || planted_bug_fix(spec.vfs_path).expect("a planted bug names its fix").trim_start_matches('/');
    match &spec.expected {
        Expected::Proves => json!({ "kind": "proves" }),
        Expected::DerivationNotCertified => json!({ "kind": "derivation_not_certified" }),
        Expected::CompilesToFol(needles) => json!({ "kind": "fol", "needles": needles }),
        Expected::OutputContains(needles) => json!({ "kind": "output_contains", "needles": needles }),
        Expected::OutputExact(output) => json!({ "kind": "output_exact", "output": output }),
        Expected::NativeOnlyCompiles => json!({
            "kind": "native_only",
            "needs": compiled_runtime_needs(spec.vfs_path).expect("a native-only program says what it needs"),
        }),
        Expected::WebPage => json!({ "kind": "web_app" }),
        Expected::KernelAllStatementsOk => json!({ "kind": "kernel_ok" }),
        Expected::KernelAdmitsAt(marker) => json!({ "kind": "kernel_admits", "marker": marker }),
        Expected::TheoryAllVerified => json!({ "kind": "theory_verified" }),
        Expected::SvaSynthesizes => json!({ "kind": "sva" }),
        Expected::SignalPlanSynthesizes => json!({ "kind": "signal_plan" }),
        Expected::RtlProven => json!({ "kind": "rtl_proven" }),
        Expected::RefutesWithCounterexample => json!({ "kind": "rtl_refuted", "fixed": fixed() }),
        Expected::RegisterAllocFits => json!({ "kind": "regalloc_fits" }),
        Expected::SpillsRequired => json!({ "kind": "regalloc_spills", "fixed": fixed() }),
        Expected::UnsatCertified => json!({ "kind": "unsat_certified" }),
    }
}

fn mode_name(mode: StudioMode) -> &'static str {
    match mode {
        StudioMode::Logic => "logic",
        StudioMode::Code => "code",
        StudioMode::Math => "math",
        StudioMode::Hardware => "hardware",
        StudioMode::Web => "web",
    }
}

fn render_contract_json() -> String {
    let examples: Vec<serde_json::Value> = ALL_LOGIC_EXAMPLES
        .iter()
        .chain(ALL_CODE_EXAMPLES)
        .chain(ALL_MATH_EXAMPLES)
        .chain(ALL_HARDWARE_SPECS)
        .map(|spec| {
            let opens_in = StudioMode::for_path(spec.vfs_path).expect("every example opens in a mode");
            serde_json::json!({
                "file": spec.vfs_path.trim_start_matches('/'),
                "mode": mode_name(opens_in),
                "promise": promise(spec),
            })
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&serde_json::json!({ "examples": examples }))
        .expect("the contract serializes");
    text.push('\n');
    text
}

#[test]
fn the_studio_example_contract_matches_the_registry() {
    let path = contract_path();
    if std::env::var_os("LOGOS_REGEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("fixtures directory")).expect("create fixtures directory");
        std::fs::write(&path, render_contract_json()).expect("write the contract");
    }
    let have = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing — LOGOS_REGEN=1 cargo nextest run -p logicaffeine-tests --features web-tests --test studio_example_contract",
            path.display()
        )
    });
    assert_eq!(
        have,
        render_contract_json(),
        "the Studio example contract is stale — LOGOS_REGEN=1 cargo nextest run -p logicaffeine-tests --features web-tests --test studio_example_contract"
    );
}
