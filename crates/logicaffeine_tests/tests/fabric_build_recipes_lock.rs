#![cfg(feature = "handed-reds")]
//! fabric_build_recipes_lock — every demo image names its build, and every resource figure about it is its build's.
//!
//! FABRIC-BUILD-RECIPES's RED tests (benchmarks/FUSIONS_WE_NEED.md, section 12.20). `scripts/openxc7-build.sh` writes
//! a recipe beside each bitstream: the tool, the seed, the router, the sources and their SHA-256, and nextpnr's
//! maximum frequency per clock. Each demo image carries one whose digest is its committed sources', the recipe also
//! records nextpnr's utilisation (today the script keeps only the last four matching lines, which are the frequency
//! lines), and every LUT figure `fabric_perf.json` quotes is read from its image's recipe.

use std::path::{Path, PathBuf};

use logicaffeine_kernel::interface::lean::package::sha256_hex;

fn silicon() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// The demo images and the ledger field each one's LUT count backs.
const IMAGES: &[(&str, &str)] = &[
    ("nexys_fabric_soc.bit", "/fabric_area/slice_luts"),
    ("nexys_state_soc.bit", "/state_plane/slice_luts"),
    ("nexys_float_soc.bit", "/float_lane/slice_luts"),
    ("full_core_pf_proven.bit", "/soft_core_area/slice_luts"),
];

fn recipe(bit: &str) -> String {
    std::fs::read_to_string(silicon().join(format!("{bit}.build.txt")))
        .unwrap_or_else(|e| panic!("{bit} has no recipe ({e}): rebuild it through scripts/openxc7-build.sh"))
}

/// The recipe's sources, from its `SRC="…"`.
fn sources(recipe: &str) -> Vec<String> {
    let at = recipe.find("SRC=\"").expect("a recipe names its sources") + 5;
    let end = recipe[at..].find('"').unwrap() + at;
    recipe[at..end].split_whitespace().map(str::to_string).collect()
}

/// nextpnr's LUT count from the recipe's utilisation lines.
fn luts(recipe: &str) -> Option<u64> {
    recipe
        .lines()
        .find(|l| l.contains("SLICE_LUTX"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().split('/').next())
        .and_then(|n| n.trim().parse().ok())
}

#[test]
fn every_demo_image_has_a_recipe_that_names_its_sources() {
    for (bit, _) in IMAGES {
        let r = recipe(bit);
        let srcs = sources(&r);
        assert!(!srcs.is_empty(), "{bit}: its recipe names no sources");
        let mut all = Vec::new();
        for s in &srcs {
            all.extend(std::fs::read(silicon().join(s)).unwrap_or_else(|e| panic!("{bit}: its source {s}: {e}")));
        }
        let sha = r
            .lines()
            .find_map(|l| l.strip_prefix("rtl sha256 "))
            .unwrap_or_else(|| panic!("{bit}: its recipe records no `rtl sha256` of its sources"));
        assert_eq!(sha.trim(), sha256_hex(&all), "{bit}: the committed sources are not the ones it was built from");
    }
}

#[test]
fn every_recipe_records_its_resources() {
    for (bit, _) in IMAGES {
        assert!(luts(&recipe(bit)).is_some(), "{bit}: its recipe records no LUT count (nextpnr's utilisation)");
    }
}

#[test]
fn every_lut_figure_in_fabric_perf_is_its_images_recipes() {
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(silicon().join("fabric_perf.json")).unwrap()).unwrap();
    for (bit, pointer) in IMAGES {
        let stated = doc.pointer(pointer).and_then(|v| v.as_u64()).unwrap_or_else(|| panic!("fabric_perf.json has no {pointer}"));
        let built = luts(&recipe(bit)).unwrap_or_else(|| panic!("{bit}: no LUT count"));
        assert_eq!(stated, built, "fabric_perf.json{pointer} says {stated} LUTs; {bit} was built with {built}");
    }
}
