//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! fabric_capacity_lock — the fabric refuses a program too big for its part before a line of RTL is printed.
//!
//! A `SpatialTarget` that names a part ([`SpatialTarget::part`]) carries what that part holds — its block RAMs,
//! UltraRAMs, LUT-RAM, flip-flops and DSP slices — and the lowering spends them as a budget. The budget counts
//! exactly the memories the printer declares, each resource a LOWER bound (the fewest the declared memories can
//! take), so a refusal is sound: the design needs at least this much and this much already does not fit. This
//! lock holds the budget to the memories the printer emits, refuses a weight past the part's block RAM with a
//! message that names the part and the resource, and leaves a program that fits unrefused.
//!
//!   cargo nextest run -p logicaffeine-tests --test fabric_capacity_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::fabric::budget::{fabric_memories, fabric_usage, DeclaredMemory};
use logicaffeine_compile::spatial::lower::fabric::FabricBackend;
use logicaffeine_compile::spatial::lower::{FabricResource, SpatialArtifact, SpatialBackend, SpatialLowerError};
use logicaffeine_compile::spatial::part::MemoryKind;
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_program_to_spatial_returning};
use logicaffeine_compile::spatial::target::SpatialTarget;

/// A `rows × cols` Int tensor named `name`, each listed `(1-based index, value)` set; every other entry is zero.
/// A couple of entries at the int8 extremes make its value range 8 bits without a big source.
fn tensor(name: &str, rows: usize, cols: usize, set: &[(usize, i64)]) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
    for (i, v) in set {
        s.push_str(&format!("Set item {i} of {name} to {v}.\n"));
    }
    s
}

fn lowered(label: &str, src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).unwrap_or_else(|e| panic!("{label}: a tensor program: {e:?}"));
    l.program.unwrap_or_else(|| panic!("{label}: lowers whole: {:?}", l.missing.first()))
}

/// A matrix-vector product whose weight is `rows × cols`, the int8 extremes set so the weight is 8 bits wide.
fn matvec(rows: usize, cols: usize) -> String {
    format!(
        "## Main\n{}{}Let C be A matmul W.\n",
        tensor("A", 1, rows, &[(1, -128), (2, 127)]),
        tensor("W", rows, cols, &[(1, -128), (2, 127), (rows * cols, -5)]),
    )
}

// ── The refusal ──────────────────────────────────────────────────────────────────────────────────────

/// A 768×1024 int8 weight holds 6,291,456 bits; in the 100T's 8 banks that is 342 of its 18Kb RAMB18 halves,
/// past the 270 the part has. The lowering refuses it before emitting any RTL, naming the part, the resource and
/// the memory that needs the most.
#[test]
fn a_tensor_past_the_parts_block_ram_is_refused_before_a_line_is_printed() {
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    assert_eq!(target.site.mem_banks, 8);
    let p = lowered("big matvec", &matvec(768, 1024), &target);
    match FabricBackend::new(target.clone()).lower(&p) {
        Err(SpatialLowerError::OverCapacity(r)) => {
            assert_eq!(r.part, "xc7a100t");
            let block = r
                .over
                .iter()
                .find(|o| o.resource == FabricResource::BlockRam18)
                .expect("the block RAM is the resource it overruns");
            assert_eq!(block.have, 270, "the 100T has 135 × 2 RAMB18 halves");
            assert!(block.need > block.have, "it needs {} of {}", block.need, block.have);
            assert!(block.need >= 342, "a 768×1024 int8 weight needs at least 342 RAMB18, got {}", block.need);
            let (name, bits) = block.largest.first().expect("the refusal names the largest memory");
            assert!(name.contains("mem_t"), "the largest is a tensor memory, got {name}");
            assert!(*bits >= 6_291_456, "the weight holds at least 768×1024×8 bits, got {bits}");
        }
        other => panic!("a weight past the part's block RAM must be refused, got {other:?}"),
    }
}

/// The budget is read from the part, not hardcoded: the same 768×1024 weight the 100T refuses (270 RAMB18) the
/// 200T holds (730 RAMB18, 16 banks), because the lowering reads each part's own block-RAM count.
#[test]
fn the_budget_is_read_from_the_part_not_hardcoded() {
    let t100 = SpatialTarget::xc7a100t_fabric(16, 64);
    let t200 = SpatialTarget::xc7a200t_fabric(16, 64);
    let p100 = lowered("w", &matvec(768, 1024), &t100);
    let p200 = lowered("w", &matvec(768, 1024), &t200);
    let have = |t: &SpatialTarget| {
        t.part
            .expect("an xc7 preset names its part")
            .memory(logicaffeine_compile::spatial::part::MemoryKind::Block36)
            .expect("block RAM")
            .count
            .value
            * 2
    };
    assert_eq!(have(&t100), 270);
    assert_eq!(have(&t200), 730);
    // The 100T refuses it; the 200T, with more block RAM and more banks, holds the same weight.
    assert!(matches!(FabricBackend::new(t100).lower(&p100), Err(SpatialLowerError::OverCapacity(_))));
    match FabricBackend::new(t200).lower(&p200) {
        Ok(SpatialArtifact::Verilog(_)) => {}
        other => panic!("the 200T holds a 768×1024 weight, got {other:?}"),
    }
}

/// A program that fits is lowered, not refused — the check does not stand in the way of a real design.
#[test]
fn a_program_that_fits_is_not_refused() {
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let p = lowered("small matvec", &matvec(8, 12), &target);
    match FabricBackend::new(target).lower(&p) {
        Ok(SpatialArtifact::Verilog(_)) => {}
        other => panic!("a small matmul fits the 100T and must lower, got {other:?}"),
    }
}

/// An abstract target — one that names no part — is never refused here, however large its program.
#[test]
fn a_target_with_no_part_is_never_refused_for_capacity() {
    let target = SpatialTarget::sky_fabric(1, 64);
    assert!(target.part.is_none(), "the sky fabric is abstract");
    let p = lowered("big on sky", &matvec(768, 1024), &target);
    assert!(
        !matches!(FabricBackend::new(target).lower(&p), Err(SpatialLowerError::OverCapacity(_))),
        "a target with no part has no capacity to overrun"
    );
}

// ── The budget counts what the printer declares ───────────────────────────────────────────────────────

/// Every memory the budget counts is a memory the printer declares, at the rows and width the budget used: for
/// each `mem_t{id}`, the RTL carries `reg [width-1:0] mem_t{id}_b0 [0:rows-1];` (bank 0, lane 0), with
/// rows = ⌈words/banks⌉.
#[test]
fn the_budget_counts_the_memories_the_printer_declares() {
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let p = lowered("matvec", &matvec(8, 12), &target);
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("it lowers: {other:?}"),
    };
    let mems = fabric_memories(&p, &target).expect("the budget reads the memories");
    assert!(mems.iter().any(|m| m.name.starts_with("mem_t")), "there are tensor memories");
    for m in mems.iter().filter(|m| m.name.starts_with("mem_t")) {
        let rows = m.words.max(1).div_ceil(m.banks);
        // The first lane of bank 0 of this memory: `reg [..] <name>_b0 [0:rows-1];` (one bank => no `_b0`? the
        // printer always suffixes the bank). Hold the declaration's depth to the budget's rows.
        let decl = format!("{}_b0", m.name);
        let line = art
            .rtl
            .lines()
            .find(|l| l.contains(&decl) && l.contains("reg ") && l.contains(&format!("[0:{}]", rows - 1)))
            .unwrap_or_else(|| panic!("the RTL declares {} with {rows} rows; none found", m.name));
        assert!(line.contains("reg "), "{} is a reg array: {line}", m.name);
    }
}

/// Every memory array the printed RTL declares, as `(memory, rows, bits)`: a site's `mem_t<id>` and a pool module's
/// `mem` (named for its module, `pool_<x>_<y>_<i>`), each summed over its banks and lanes from the declarations
/// `reg [W-1:0] <memory>[_b<bank>][_l<lane>] [0:R-1];`.
fn declared_memories(rtl: &str) -> Vec<(String, usize, u64)> {
    let mut module = String::new();
    let mut found: BTreeMap<(String, String), (BTreeSet<usize>, u64)> = BTreeMap::new();
    for line in rtl.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("module ") {
            module = rest.split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or_default().to_string();
            continue;
        }
        let Some(at) = line.find("reg [") else { continue };
        let decl = &line[at + "reg [".len()..];
        let Some((msb, rest)) = decl.split_once(":0] ") else { continue };
        let Some((name, rest)) = rest.split_once(" [0:") else { continue };
        let Some(last) = rest.strip_suffix("];") else { continue };
        let (Ok(msb), Ok(last)) = (msb.parse::<u64>(), last.parse::<usize>()) else { continue };
        let mut stem = name;
        for suffix in ["_l", "_b"] {
            if let Some((head, tail)) = stem.rsplit_once(suffix) {
                if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
                    stem = head;
                }
            }
        }
        let memory = match stem {
            "mem" => module.clone(),
            s if s.strip_prefix("mem_t").is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) => s.to_string(),
            _ => continue,
        };
        let entry = found.entry((module.clone(), memory)).or_default();
        entry.0.insert(last + 1);
        entry.1 += (msb + 1) * (last as u64 + 1);
    }
    let mut out: Vec<(String, usize, u64)> = found
        .into_iter()
        .map(|((module, memory), (rows, bits))| {
            assert_eq!(rows.len(), 1, "every bank and lane of {memory} in {module} has one depth, got {rows:?}");
            (memory, *rows.first().unwrap(), bits)
        })
        .collect();
    out.sort();
    out
}

/// The budget's memories, as `(memory, rows, bits)` in the order [`declared_memories`] reads the RTL.
fn budgeted_memories(p: &SpatialProgram, target: &SpatialTarget) -> Vec<(String, usize, u64)> {
    let mut out: Vec<(String, usize, u64)> = fabric_memories(p, target)
        .expect("the budget reads the memories")
        .iter()
        .map(|m| (m.name.clone(), m.words.max(1).div_ceil(m.banks), m.bits()))
        .collect();
    out.sort();
    out
}

/// The budget is the printer's memories exactly — the same memories, each at the depth and the bits the RTL declares,
/// none missing and none extra — on every layout the printer makes: a weight whose rows are whole words, one whose rows
/// of twelve are kept at a pitch of sixteen, and both decode images (Board A at eight lanes, whose tied head's rows of
/// fourteen take a pitch and whose gathers read through a port a lane; Board B at four), as the images lower them.
#[test]
fn the_budget_is_every_memory_the_printer_declares_at_its_depth_and_bits() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tiny_transformer.json")).expect("json");
    let model = Int8Model::from_json(&j).expect("the model");
    let step = model.decode_step_lg(0, 0, &[], &[]);
    let board = |mut target: SpatialTarget, lanes: usize| {
        target.site.vector_lanes = lanes;
        let l = compile_program_to_spatial_returning(&step, &target, &Int8Model::step_reads()).expect("the decode step");
        (l.program.expect("the decode step lowers whole"), target)
    };
    let a100 = SpatialTarget::xc7a100t_fabric(16, 64);
    let cases: Vec<(&str, SpatialProgram, SpatialTarget)> = vec![
        ("whole-word rows", lowered("w16", &matvec(8, 16), &a100), a100.clone()),
        ("rows of twelve", lowered("w12", &matvec(8, 12), &a100), a100.clone()),
        {
            let (p, t) = board(SpatialTarget::xc7a200t_fabric(16, 64), 8);
            ("Board A's decode step", p, t)
        },
        {
            let (p, t) = board(SpatialTarget::xc7a100t_fabric(16, 64), 4);
            ("Board B's decode step", p, t)
        },
    ];
    let mut pitched = 0;
    for (label, p, target) in cases {
        let rtl = match FabricBackend::new(target.clone()).lower(&p) {
            Ok(SpatialArtifact::Verilog(v)) => v.rtl,
            other => panic!("{label}: lowers: {other:?}"),
        };
        let printed = declared_memories(&rtl);
        let budget = budgeted_memories(&p, &target);
        assert!(printed.iter().any(|(m, ..)| m.starts_with("mem_t")), "{label}: the RTL declares tensor memories");
        assert!(printed.iter().any(|(m, ..)| m.starts_with("pool_")), "{label}: the RTL declares pools");
        assert_eq!(budget, printed, "{label}: the budget's (memory, rows, bits) are the RTL's");
        let unpitched: u64 = p
            .tensors
            .iter()
            .map(|t| (t.tiles.rows_t * t.tiles.cols_t).max(1).div_ceil(target.site.mem_banks) as u64)
            .sum();
        let rows: u64 = budget.iter().filter(|(m, ..)| m.starts_with("mem_t")).map(|(_, r, _)| *r as u64).sum();
        pitched += usize::from(rows > unpitched);
    }
    assert!(pitched >= 2, "the cases include tensors kept at a pitch (the rows of twelve, the tied head), got {pitched}");
}

/// The budget's RAMB18 count is the block-styled memories' bits over a RAMB18's 18,432, computed here from the
/// memory list independently of the budget. An independent recomputation of the block bits divides to the same
/// RAMB18 the budget reports.
#[test]
fn the_ramb18_count_is_the_block_memories_bits_over_a_ramb18() {
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let p = lowered("big matvec", &matvec(768, 1024), &target);
    let u = fabric_usage(&p, &target).expect("usage");
    let mems = fabric_memories(&p, &target).expect("memories");
    assert_eq!(u.memories, mems, "the usage carries the memory list it counted");
    // Replicate Storage::of's block test independently: not latency 0, and rows past the distributed ceiling.
    let banks = target.site.mem_banks;
    let block_bits: u64 = mems
        .iter()
        .filter(|m| {
            let rows = m.words.max(1).div_ceil(m.banks);
            let registers = rows <= 1 || (m.banks == 1 && rows <= 4);
            let distributed = rows <= 64 && m.banks > 1;
            m.lat != 0 && !registers && !distributed
        })
        .map(DeclaredMemory::bits)
        .sum();
    assert!(block_bits > 0, "a 768×1024 weight is block RAM at {banks} banks");
    assert_eq!(u.ramb18, block_bits.div_ceil(18_432), "the budget's RAMB18 is the block bits over a RAMB18");
}

/// A matmul spends DSP slices — at least one a multiply-accumulate of its array; a program with no matmul spends
/// none. The budget counts them both ways.
#[test]
fn the_dsp_budget_is_a_mac_an_array_cell() {
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let matmul = lowered("matmul", &matvec(8, 12), &target);
    let dsp = fabric_usage(&matmul, &target).expect("usage").dsp;
    assert!(dsp > 0, "a matmul spends DSP slices, got {dsp}");
    assert!(dsp <= target.part.unwrap().dsp.value, "its DSP {dsp} is within the part");

    // A requant is elementwise — no matmul array, no DSP.
    let requant = format!(
        "## Main\n{}Let Y be X times 104593 shifted right by 24.\n",
        tensor("X", 1, 13, &[(1, -128), (2, 127)]),
    );
    let p = lowered("requant", &requant, &target);
    assert_eq!(fabric_usage(&p, &target).expect("usage").dsp, 0, "a program with no matmul spends no DSP");
}

/// A distributed-RAM memory is a LUT copy PER READ PORT: a pool a gather's lanes read through a port a lane costs
/// its bits that many times. Board A's decode step at 8 lanes reads the exp table's page through 8 ports, so the
/// LUT-RAM bound counts it eight times — more than one copy would — and the budget's figure is exactly the
/// per-memory ⌈bits/64⌉ × ports summed. (Dropping the ports makes it fall back to one copy, under the real cost.)
#[test]
fn the_lut_ram_bound_counts_a_lut_copy_per_read_port() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tiny_transformer.json")).expect("json");
    let model = Int8Model::from_json(&j).expect("the model");
    let step = model.decode_step_lg(0, 0, &[], &[]);
    let mut target = SpatialTarget::xc7a200t_fabric(16, 64);
    target.site.vector_lanes = 8;
    let l = compile_program_to_spatial_returning(&step, &target, &Int8Model::step_reads()).expect("the decode step");
    let p = l.program.expect("the decode step lowers whole");
    let u = fabric_usage(&p, &target).expect("usage");

    let is_distributed = |m: &DeclaredMemory| {
        let rows = m.words.max(1).div_ceil(m.banks);
        m.lat != 0 && !(rows <= 1 || (m.banks == 1 && rows <= 4)) && rows <= 64 && m.banks > 1
    };
    let per_port: u64 = u.memories.iter().filter(|m| is_distributed(m)).map(|m| m.bits().div_ceil(64) * m.read_ports as u64).sum();
    let one_copy: u64 = u.memories.iter().filter(|m| is_distributed(m)).map(|m| m.bits().div_ceil(64)).sum();
    assert_eq!(u.lutram_luts, per_port, "the LUT-RAM bound is a LUT copy per read port");
    assert!(
        u.memories.iter().any(|m| is_distributed(m) && m.name.starts_with("pool_") && m.read_ports > 1),
        "Board A's 8 lanes read a pool through more than one port"
    );
    assert!(u.lutram_luts > one_copy, "the per-lane copies cost more than one: {} vs {one_copy}", u.lutram_luts);
    assert!(
        u.lutram_luts <= target.part.unwrap().memory(MemoryKind::LutRam).unwrap().count.value,
        "the bound is within the part's LUT-RAM"
    );
}

/// Board A (Nexys Video, xc7a200t) routed the decode image with ZERO block RAM — every tensor fits its 16 banks
/// in LUT-RAM (`benchmarks/silicon/decode_images.json`: `RAMB18E1_RAMB18E1 used 0`). The budget agrees: on the
/// 200T the decode step needs no RAMB18 and no UltraRAM, and its LUT-RAM is within the part — so the bound never
/// claims a resource the routed image did not use, and the step is not refused.
#[test]
fn the_decode_step_needs_no_block_ram_on_the_16_bank_part_as_the_routed_image_used_none() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tiny_transformer.json")).expect("json");
    let model = Int8Model::from_json(&j).expect("the model");
    let src = model.decode_step_lg(0, 0, &[], &[]);
    let target = SpatialTarget::xc7a200t_fabric(16, 64);
    let p = lowered("decode step", &src, &target);
    let u = fabric_usage(&p, &target).expect("usage");
    assert_eq!(u.ramb18, 0, "the decode fits the 200T's 16 banks in LUT-RAM, as the routed Board A image did (0 BRAM)");
    let lutram_have = target.part.unwrap().memory(MemoryKind::LutRam).unwrap().count.value;
    assert!(u.lutram_luts <= lutram_have, "its LUT-RAM {} is within the part's {lutram_have}", u.lutram_luts);
    match FabricBackend::new(target).lower(&p) {
        Ok(SpatialArtifact::Verilog(_)) => {}
        other => panic!("the decode step fits the 200T and must lower, got {other:?}"),
    }
}
