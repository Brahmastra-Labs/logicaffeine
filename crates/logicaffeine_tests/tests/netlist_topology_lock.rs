//! netlist_topology_lock — the SoC's TOPOLOGY (which block contains/issues which) is proven
//! against the real module-instantiation graph extracted from the synthesized RTL, so the
//! containment it records can never be hand-drawn fiction.
//!
//! Source of truth: `benchmarks/silicon/netlist_topology.json`, written by
//! `scripts/extract-netlist-topology.py` (parses `benchmarks/silicon/*.v`, testbenches
//! excluded, for `MODNAME instance(` instantiations of other design modules).
//!
//! The per-block synthesis reports (`reports.json`) pin every block's NUMBERS; this pins the
//! STRUCTURE: the blocks are real synthesized modules AND the edges between them are real
//! instantiations. When RED after an RTL change, re-extract
//! (`python3 scripts/extract-netlist-topology.py`).

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> serde_json::Value {
    let p = root().join(rel);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — run scripts/extract-netlist-topology.py",
            p.display()
        )
    });
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {rel}: {e}"))
}

const TOPO: &str = "benchmarks/silicon/netlist_topology.json";

/// True iff module `parent` instantiates module `child` in the extracted graph.
fn instantiates(d: &serde_json::Value, parent: &str, child: &str) -> bool {
    d["graph"][parent]["instantiates"]
        .as_object()
        .map(|m| m.contains_key(child))
        .unwrap_or(false)
}

fn module_exists(d: &serde_json::Value, name: &str) -> bool {
    d["modules"]
        .as_array()
        .map(|a| a.iter().any(|v| v == name))
        .unwrap_or(false)
}

#[test]
fn the_diagram_containment_edges_are_real_instantiations() {
    let d = json(TOPO);

    // Each (parent, child) is a containment the SoC diagram draws; it MUST be a real
    // instantiation in the synthesized RTL. These are the structural spine of the diagram:
    // the CPU datapath, the coprocessor issue block, the flashable SoC, the SHA-3 hierarchy,
    // and the reversible core.
    let edges: &[(&str, &str)] = &[
        ("soft_core", "alu"),            // the CPU's integer ALU
        ("soft_core", "shared_divider"), // the shared iterative divider (W2) is really wired in
        ("soft_core_scalar", "alu"),     // the routable scalar core shares the ALU
        ("coproc_core", "bigint_alu"), // "coprocessors ISSUED as instructions" — really instantiated
        ("coproc_core", "bigint_mul"),
        ("coproc_core", "crdt_merge"),
        ("coproc_soc", "coproc_core"), // the flashable SoC wraps the coproc core
        ("keccak_top", "keccak_core"), // SHA-3 hierarchy
        ("keccak_core", "keccak_round"),
        ("rev_core", "rev_alu_core"), // the reversible datapath
    ];
    for (parent, child) in edges {
        assert!(
            module_exists(&d, parent) && module_exists(&d, child),
            "{TOPO}: diagram edge {parent}->{child} names a module not in the netlist"
        );
        assert!(
            instantiates(&d, parent, child),
            "{TOPO}: the diagram shows {parent} containing {child}, but the RTL that \
             synthesizes does NOT instantiate it — the diagram drifted from the silicon"
        );
    }
}

#[test]
fn every_block_the_diagram_names_is_a_real_module() {
    let d = json(TOPO);
    // The named blocks the diagram labels REAL, each of which must be a real synthesized module.
    for block in [
        "soft_core_scalar",
        "rev_core",
        "fabric_horner",
        "crdt_merge",
        "chan_fifo",
        "bigint_alu",
        "bigint_mul",
        "bigint_div",
        "shared_divider",
        "dram_executor",
        "secded_enc",
        "secded_dec",
        "double_buffer",
        "keccak_core",
        "ntt_butterfly",
        "coproc_core",
        "coproc_soc",
    ] {
        assert!(
            module_exists(&d, block),
            "{TOPO}: the diagram names `{block}` REAL, but it is not a module in the netlist \
             — extract is stale or the block was renamed"
        );
    }
}

/// True iff `src` contains an instantiation of module `child`: `child [#(...)] instname (`.
fn source_instantiates(src: &str, child: &str) -> bool {
    let id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let b = src.as_bytes();
    let mut from = 0;
    while let Some(i) = src[from..].find(child) {
        let s = from + i;
        let e = s + child.len();
        from = s + 1;
        // word boundary around the module name
        if s > 0 && id(b[s - 1] as char) {
            continue;
        }
        if e < b.len() && id(b[e] as char) {
            continue;
        }
        // after the name: whitespace, an optional #(...) param block, an instance name, then `(`
        let mut rest = src[e..].trim_start();
        if let Some(after_hash) = rest.strip_prefix('#') {
            let ah = after_hash.trim_start();
            if !ah.starts_with('(') {
                continue;
            }
            // Skip a BALANCED paren group — the param list can nest, e.g. `#(.W(32))`.
            let mut depth = 0i32;
            let mut close = None;
            for (i, c) in ah.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match close {
                Some(i) => rest = ah[i + 1..].trim_start(),
                None => continue,
            }
        }
        // now expect: instance_name (
        let inst_len = rest.chars().take_while(|&c| id(c)).count();
        if inst_len == 0 {
            continue;
        }
        if rest[inst_len..].trim_start().starts_with('(') {
            return true;
        }
    }
    false
}

#[test]
fn the_committed_topology_is_fresh_against_the_live_rtl() {
    // The committed JSON could go stale if RTL changes without re-running the extractor; guard
    // the load-bearing edges by reading the parent's actual .v and confirming the instantiation
    // is really there. (The divider edge is the W2 integration; the coproc edges are the
    // "coprocessors as instructions" claim — both must never silently un-wire.)
    let d = json(TOPO);
    let edges: &[(&str, &str)] = &[
        ("soft_core", "alu"),
        ("soft_core", "shared_divider"),
        ("coproc_core", "bigint_alu"),
        ("coproc_core", "bigint_mul"),
        ("coproc_core", "crdt_merge"),
        ("coproc_soc", "coproc_core"),
        ("keccak_top", "keccak_core"),
    ];
    for (parent, child) in edges {
        let file = d["graph"][parent]["file"]
            .as_str()
            .unwrap_or_else(|| panic!("{TOPO}: no file recorded for {parent}"));
        let src = fs::read_to_string(root().join("benchmarks/silicon").join(file))
            .unwrap_or_else(|e| panic!("read {file}: {e}"));
        assert!(
            source_instantiates(&src, child),
            "netlist_topology.json says {parent}->{child}, but {file} does not instantiate \
             {child} — the extract is STALE; re-run scripts/extract-netlist-topology.py"
        );
    }
}

#[test]
fn the_extracted_graph_is_well_formed() {
    let d = json(TOPO);
    let mods = d["modules"].as_array().expect("modules[]");
    assert!(
        mods.len() >= 20,
        "{TOPO}: only {} modules — extraction looks broken",
        mods.len()
    );

    // Every instantiated child in the graph must itself be a declared module (no dangling
    // instances), and no module instantiates itself.
    let graph = d["graph"].as_object().expect("graph{}");
    for (parent, entry) in graph {
        if let Some(insts) = entry["instantiates"].as_object() {
            for child in insts.keys() {
                assert!(
                    module_exists(&d, child),
                    "{TOPO}: {parent} instantiates unknown module {child}"
                );
                assert_ne!(parent, child, "{TOPO}: {parent} instantiates itself");
            }
        }
    }
}
