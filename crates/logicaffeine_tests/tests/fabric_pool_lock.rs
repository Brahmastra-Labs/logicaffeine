//! fabric_pool_lock — a fabric site keeps its FIFO pages in pools: as many page memories as its busiest phase has
//! buffers, not one per buffer of the program.
//!
//! A FIFO is its control — counts, reservations, the slot it reads — and its pages live in one of its site's pools.
//! Every engine of a site passes a rendezvous between two section phases and every op's reads and writes land within its
//! own cycles, so buffers of different phases never move data in one cycle: each phase's buffers, largest first, take
//! pools 0, 1, 2, …, and a pool's ports take its members' requests merged. The decode step's 115 buffers over 38 phases
//! keep their pages in as many pools as its busiest phase has buffers. This lock holds:
//!
//! - on every fabric target, the decode step's site has exactly as many pool memories as its busiest phase has
//!   buffers, and no FIFO module holds a memory of its own;
//! - a program of one section is one phase: a pool per buffer;
//! - a pool shared by more than one buffer stops its simulation loudly if two of them write in one cycle.
//!
//! The pooled RTL equals the evaluator in exactly the twin's cycles: `fabric_banks_lock`, `fabric_integer_lock` and
//! `fabric_pipeline_lock` run the programs this lock counts.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_pool_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{BufferKind, SpatialProgram};
use logicaffeine_compile::spatial::lower::fabric::{FabricBackend, VerilogArtifact};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

fn fabrics() -> [SpatialTarget; 3] {
    [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64), SpatialTarget::xc7a100t_fabric(16, 64)]
}

fn decode_step() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[])
}

fn lowered(label: &str, src: &str, target: &SpatialTarget) -> (SpatialProgram, VerilogArtifact) {
    let l = compile_program_to_spatial(src, target).unwrap_or_else(|e| panic!("{label}: a tensor program: {e:?}"));
    let p = l.program.unwrap_or_else(|| panic!("{label}@{}: lowers whole: {:?}", target.name, l.missing.first()));
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("{label}@{}: the fabric lowers it: {other:?}", target.name),
    };
    (p, art)
}

/// Each module of `rtl` by name, with its text.
fn modules(rtl: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in rtl.lines() {
        if let Some(rest) = line.strip_prefix("module ") {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            current = Some((name, String::new()));
        }
        if let Some((_, text)) = current.as_mut() {
            text.push_str(line);
            text.push('\n');
        }
        if line.starts_with("endmodule") {
            if let Some((name, text)) = current.take() {
                out.insert(name, text);
            }
        }
    }
    out
}

/// The most page FIFOs any one section phase of `p` has, or all of them where no section splits the program.
fn busiest_phase(p: &SpatialProgram) -> usize {
    let phases = p.section_phases();
    let mut by: BTreeMap<Option<usize>, usize> = BTreeMap::new();
    for b in p.buffers.iter().filter(|b| !matches!(b.kind, BufferKind::Accumulator { .. })) {
        *by.entry(p.section_of_buffer(b.id).map(|k| phases[k])).or_default() += 1;
    }
    by.values().copied().max().unwrap_or(0)
}

fn fifos(p: &SpatialProgram) -> usize {
    p.buffers.iter().filter(|b| !matches!(b.kind, BufferKind::Accumulator { .. })).count()
}

/// Whether a module declares a memory array.
fn holds_memory(module: &str) -> bool {
    module.lines().any(|l| {
        let t = l.trim_start();
        (t.starts_with("reg ") || t.contains(") reg ")) && t.contains(" [0:")
    })
}

#[test]
fn the_decode_step_keeps_its_pages_in_as_many_pools_as_its_busiest_phase_has_buffers() {
    for target in fabrics() {
        let (p, art) = lowered("decode step", &decode_step(), &target);
        let m = modules(&art.rtl);
        let pools = m.keys().filter(|n| n.starts_with("pool_")).count();
        let busiest = busiest_phase(&p);
        assert!(p.sections.len() > 1 && busiest < fifos(&p), "{}: the step's phases share pools ({busiest} of {} buffers)", target.name, fifos(&p));
        assert_eq!(pools, busiest, "{}: a pool for each buffer of the busiest phase", target.name);
        for (name, text) in m.iter().filter(|(n, _)| n.starts_with("fifo_tpl_")) {
            assert!(!holds_memory(text), "{}: {name} keeps no memory of its own", target.name);
        }
        for (name, text) in m.iter().filter(|(n, _)| n.starts_with("pool_")) {
            assert!(holds_memory(text), "{}: {name} is a memory", target.name);
        }
    }
}

#[test]
fn a_program_of_one_section_keeps_a_pool_per_buffer() {
    let x: String = (1..=13).map(|i| format!("Set item {i} of X to {}.\n", i * 997 - 6000)).collect();
    let src = format!("## Main\nLet X be a 1 by 13 tensor of Int.\n{x}Let Y be X times 104593 shifted right by 24.\n");
    for target in fabrics() {
        let (p, art) = lowered("requant", &src, &target);
        assert!(p.sections.len() <= 1, "{}: one op, one phase", target.name);
        let pools = modules(&art.rtl).keys().filter(|n| n.starts_with("pool_")).count();
        assert_eq!(pools, fifos(&p), "{}: every buffer of one phase in a pool of its own", target.name);
    }
}

#[test]
fn a_shared_pool_stops_its_simulation_if_two_buffers_write_at_once() {
    for target in fabrics() {
        let (_, art) = lowered("decode step", &decode_step(), &target);
        let guards = art.rtl.matches("two of its buffers write in one cycle").count();
        assert!(guards > 0, "{}: the step's shared pools guard their write port", target.name);
        assert!(art.rtl.contains("// synthesis translate_off"), "{}: the guard is the simulation's alone", target.name);
    }
}

#[test]
fn the_module_scan_reads_modules_as_they_are() {
    let rtl = "module fifo_tpl_3(input clk);\n  reg [3:0] cnt;\nendmodule\nmodule pool_0_0_1(input clk);\n  (* ram_style = \"block\" *) reg [7:0] mem_b0 [0:63];\nendmodule\n";
    let m = modules(rtl);
    assert_eq!(m.len(), 2);
    assert!(!holds_memory(&m["fifo_tpl_3"]));
    assert!(holds_memory(&m["pool_0_0_1"]));
}
