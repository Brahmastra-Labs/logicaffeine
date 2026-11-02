//! Byte-lock for `public/examples.json` — the machine-readable example corpus.
//!
//! Exports the Syntax Guide's example registry (`guide::content::SECTIONS`)
//! — the corpus whose expected outputs are already runtime-locked by
//! `guide_examples.rs` — as input→expected-output pairs. This is few-shot
//! gold for agents learning LOGOS. Regenerate with
//! `./scripts/release/gen-agent-artifacts.sh`.

#![cfg(feature = "web-tests")]

use std::path::PathBuf;

use logicaffeine_web::ui::pages::guide::content::{ExampleMode, Runnability, SECTIONS};

/// Must equal `guide_examples.rs::EXAMPLE_CENSUS` — the same growth ratchet:
/// a new example updates the census in both files or both go red.
const EXAMPLE_CENSUS: usize = 173;

fn export_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/logicaffeine_web/public/examples.json")
}

fn render_examples_json() -> String {
    let mut items: Vec<serde_json::Value> = Vec::new();
    for section in SECTIONS.iter() {
        for example in section.examples.iter() {
            let mode = match example.mode {
                ExampleMode::Logic => "logic",
                ExampleMode::Imperative => "imperative",
                ExampleMode::Theorem => "theorem",
                ExampleMode::Web => "web",
            };
            let (runnability, note): (&str, Option<&str>) = match &example.runnability {
                Runnability::Playground => ("playground", None),
                Runnability::CompiledOnly { reason } => ("compiled-only", Some(reason)),
                Runnability::ExpectedError { message } => ("expected-error", Some(message)),
            };
            items.push(serde_json::json!({
                "id": example.id,
                "section": section.id,
                "section_title": section.title,
                "label": example.label,
                "mode": mode,
                "runnability": runnability,
                "note": note,
                "code": example.code,
                "expected_output": example.expected_output,
            }));
        }
    }
    let doc = serde_json::json!({
        "schema": "logos-examples/1",
        "census": items.len(),
        "examples": items,
    });
    let mut text = serde_json::to_string_pretty(&doc).expect("examples serialize");
    text.push('\n');
    text
}

#[test]
fn examples_json_matches_registry() {
    let want = render_examples_json();
    let path = export_path();
    let have = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing — run ./scripts/release/gen-agent-artifacts.sh",
            path.display()
        )
    });
    assert_eq!(
        have, want,
        "examples.json is stale — run ./scripts/release/gen-agent-artifacts.sh"
    );
}

#[test]
fn examples_json_census_matches_guide_census() {
    let total: usize = SECTIONS.iter().map(|s| s.examples.len()).sum();
    assert_eq!(
        total, EXAMPLE_CENSUS,
        "guide example census changed — update EXAMPLE_CENSUS here AND in \
         guide_examples.rs, then regenerate examples.json"
    );

    let text = std::fs::read_to_string(export_path())
        .expect("examples.json exists (run ./scripts/release/gen-agent-artifacts.sh)");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(doc["census"], EXAMPLE_CENSUS, "exported census must match");
    assert_eq!(
        doc["examples"].as_array().unwrap().len(),
        EXAMPLE_CENSUS,
        "every guide example is exported"
    );
}

#[test]
#[ignore = "regenerator — run via ./scripts/release/gen-agent-artifacts.sh (LOGOS_REGEN=1)"]
fn regenerate_examples_json() {
    if std::env::var_os("LOGOS_REGEN").is_none() {
        // The full suite runs #[ignore]d tests too; without the opt-in this
        // is a no-op (the byte-locks above are the actual gate).
        eprintln!("skipping regeneration: set LOGOS_REGEN=1 (use ./scripts/release/gen-agent-artifacts.sh)");
        return;
    }
    std::fs::write(export_path(), render_examples_json()).expect("write examples.json");
}
