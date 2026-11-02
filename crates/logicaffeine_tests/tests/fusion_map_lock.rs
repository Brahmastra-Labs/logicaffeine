//! fusion_map_lock — Demo 1's fusion panel is generated from the lowerings and cannot drift from them.
//!
//! For every tiny architecture, the map records which fused symbols of the book's section 3.4 the CUDA lane's decode
//! step and the D5005 engine's program fire, and why a chip refuses. The map regenerates byte for byte
//! (`tests/fixtures/fusion_map_tiny.json`; the real checkpoints' map, `benchmarks/results/demo/fusion_map.json`, on
//! demand). Every rule the lane records has a meaning, every symbol the map names is one section 3.4 maps, and every
//! rule the lane fires has a census test that names it.

#[path = "common/fusion_map.rs"]
mod fusion_map;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fusion_map::{cuda_cell, d5005_cell, lane_census, to_json, Cell, LANE_RULES};
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::plan::RULES;
use logicaffeine_compile::codegen_cuda::lane::{compile, LaneOptions};
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::ModelSpec;
use tiny_models::ALL;

const CTX: usize = 24;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The map of every tiny architecture, each brought up on the CUDA lane and the D5005 engine.
fn tiny_map() -> BTreeMap<String, BTreeMap<&'static str, Cell>> {
    let dir = scratch::dir("logos_fusion_map");
    let mut models = BTreeMap::new();
    for t in ALL {
        let spec = t.spec(CTX);
        let path = dir.join(format!("{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &path);
        models.insert(t.name().to_string(), cells(&spec, &path));
    }
    models
}

fn cells(spec: &ModelSpec, path: &Path) -> BTreeMap<&'static str, Cell> {
    let opts = LaneOptions { ctx: CTX, knobs: CudaKnobs::default(), prefill: 1 };
    let cuda = match compile(spec, path, &opts) {
        Ok(lane) => cuda_cell(&lane).unwrap_or_else(|e| panic!("{e}")),
        Err(e) => Cell { refused: Some(e.to_string()), ..Cell::default() },
    };
    BTreeMap::from([("cuda", cuda), ("d5005", d5005_cell(spec, path, CTX))])
}

/// The symbols section 3.4 of the book maps onto every chip.
fn book_symbols() -> BTreeSet<String> {
    let book = std::fs::read_to_string(root().join("benchmarks/FUSIONS_WE_NEED.md")).expect("the book");
    let at = book.find("### 3.4 Every fusion on every chip").expect("section 3.4");
    let end = book[at..].find("### 3.5 ").expect("section 3.5") + at;
    book[at..end]
        .lines()
        .filter_map(|l| l.strip_prefix("- [ ] **`").or_else(|| l.strip_prefix("- [x] **`")))
        .filter_map(|l| l.split('`').next())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_map_regenerates_byte_identical() {
    let json = to_json(&tiny_map());
    let fixture = root().join("crates/logicaffeine_tests/tests/fixtures/fusion_map_tiny.json");
    if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok() {
        std::fs::write(&fixture, &json).expect("write the fixture");
        return;
    }
    let pinned = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("{} ({e}): record it with LOGOS_UPDATE_FIXTURES=1", fixture.display()));
    assert_eq!(pinned, json, "the fusion map moved: the lowerings fire other fusions than the pinned map says");
}

#[test]
fn every_lane_rule_has_a_meaning() {
    let known: BTreeSet<&str> = LANE_RULES.iter().map(|(r, _)| *r).collect();
    for rule in RULES {
        assert!(known.contains(rule), "the lane's rule `{rule}` has no meaning in LANE_RULES");
    }
    for (model, chips) in tiny_map() {
        for rule in chips["cuda"].rules.keys() {
            assert!(known.contains(rule.as_str()), "{model}: the lane recorded `{rule}`, which LANE_RULES does not know");
        }
    }
}

#[test]
fn every_symbol_the_map_names_is_one_the_book_maps() {
    let book = book_symbols();
    assert!(book.len() >= 80, "section 3.4 lists {} symbols", book.len());
    for (model, chips) in tiny_map() {
        for (chip, cell) in chips {
            for sym in cell.symbols.keys() {
                assert!(book.contains(sym), "{model} on {chip} fires `{sym}`, which section 3.4 does not map");
            }
        }
    }
}

#[test]
fn every_rule_the_lane_fires_has_a_census_lock() {
    let mut fired = BTreeSet::new();
    for chips in tiny_map().values() {
        fired.extend(chips["cuda"].rules.keys().cloned());
    }
    assert!(fired.len() >= 10, "the tiny models fired only {} rules", fired.len());
    let always = ["fused_quant", "argmax_advance"];
    for rule in fired.iter().filter(|r| !always.contains(&r.as_str())) {
        assert!(
            !lane_census(rule).is_empty(),
            "the lane fires `{rule}` and no lane_compile_lock test names it"
        );
    }
}

#[test]
fn the_d5005_cells_are_the_engine_programs_of_the_one_program() {
    for (model, chips) in tiny_map() {
        let cell = &chips["d5005"];
        if cell.refused.is_some() {
            continue;
        }
        let cuda = &chips["cuda"];
        assert_eq!(cell.identity, cuda.identity, "{model}: the engine and the lane lower different programs");
        assert!(cell.symbols.contains_key("fused_sampling"), "{model}: the engine's step ends without its advance");
    }
}

/// The real checkpoints' map, written to `benchmarks/results/demo/fusion_map.json`. On demand: every scoreboard
/// checkpoint under `models/` it finds.
#[test]
#[ignore = "reads the real checkpoints: cargo nextest run -p logicaffeine-tests --test fusion_map_lock --run-ignored only"]
fn the_real_checkpoints_map_regenerates() {
    let mut models = BTreeMap::new();
    for name in ["llama3.2-1b", "gemma3-1b", "qwen3.5-0.8b", "qwen3-8b", "deepseek-v2-lite", "gpt-oss-20b", "qwen3.8-27b"] {
        let dir = root().join("models").join(name);
        let Some(gguf) = std::fs::read_dir(&dir).ok().and_then(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path())).find(|p| p.extension().is_some_and(|x| x == "gguf"))
        }) else {
            continue;
        };
        let g = Gguf::open(&gguf).unwrap_or_else(|e| panic!("{}: {e}", gguf.display()));
        let spec = ModelSpec::from_gguf(&g).unwrap_or_else(|e| panic!("{name}: {e}"));
        models.insert(name.to_string(), cells(&spec, &gguf));
    }
    let json = to_json(&models);
    let out = root().join("benchmarks/results/demo/fusion_map.json");
    if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(&out, &json).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(&out).unwrap_or_else(|e| panic!("{} ({e})", out.display()));
    assert_eq!(pinned, json, "the real checkpoints' fusion map moved");
}
