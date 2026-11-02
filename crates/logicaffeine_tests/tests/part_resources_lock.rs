//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! part_resources_lock — what each programmable part holds is its data sheet's, and the router's device agrees.
//!
//! A part's logic, flip-flops, multipliers and memories are the budget a fabric lowering spends and the capacity
//! the decode pricer reads, so every figure names its source and the lock holds each one to the table it quotes.
//! The Artix parts carry a second witness: the routed decode images record the device nextpnr placed them on,
//! site by site, and those counts must be the data sheet's own. The fabric presets that state their local memory
//! in block RAMs must state the part's block RAMs. The Stratix 10 SX 2800 is Intel's: its ALMs from the product table,
//! its M20K and MLAB blocks and its DSP blocks from the device overview.
//!
//!   cargo nextest run -p logicaffeine-tests --test part_resources_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/cited.rs"]
mod cited;

use std::path::Path;

use logicaffeine_compile::spatial::part::{LogicUnit, MemoryKind, PartResources};
use logicaffeine_compile::spatial::target::SpatialTarget;

fn of(part: &PartResources, kind: MemoryKind) -> &'static logicaffeine_compile::spatial::part::Memory {
    part.memory(kind).unwrap_or_else(|| panic!("{}: no {kind:?}", part.part))
}

#[test]
fn every_part_resource_names_its_source() {
    let parts = PartResources::catalog();
    assert!(parts.len() >= 4, "the parts");
    for p in &parts {
        cited::assert_cited(p.part, &p.facts(), p.sources);
        assert_eq!(PartResources::by_name(p.part).as_ref(), Some(p), "{} is found by its name", p.part);
        let kinds: Vec<MemoryKind> = p.memories.iter().map(|m| m.kind).collect();
        let mut unique = kinds.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), kinds.len(), "{}: one entry per kind of memory", p.part);
        for m in p.memories {
            assert!(m.count.value > 0 && m.bits.value > 0 && m.read_bits_per_cycle.value > 0, "{} {:?}", p.part, m.kind);
            assert!(m.read_bits_per_cycle.value <= m.bits.value, "{} {:?}: reads no more than it holds", p.part, m.kind);
        }
    }
    let mut names: Vec<&str> = parts.iter().map(|p| p.part).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), parts.len(), "part names are unique");
}

/// DS180's Artix-7 table, and the device the routed decode images were placed on: nextpnr-xilinx counts two LUT
/// sites and two flip-flops per LUT position in a slice, one CARRY4 per slice, a RAMB18 per 18 Kb half and a RAMB36
/// per block.
#[test]
fn the_artix_parts_are_ds180s_and_the_routers_device() {
    let a100 = PartResources::xc7a100t();
    let a200 = PartResources::xc7a200t();
    for (p, logic, flops, dsp, blocks, lutram) in [(&a100, 63_400, 126_800, 240, 135, 19_008), (&a200, 134_600, 269_200, 740, 365, 46_208)] {
        assert_eq!(p.logic_unit, LogicUnit::Lut6);
        assert_eq!((p.logic.value, p.flops.value, p.dsp.value), (logic, flops, dsp), "{}", p.part);
        assert_eq!(p.multiplier.value, (25, 18), "{}: DSP48E1", p.part);
        let block = of(p, MemoryKind::Block36);
        assert_eq!((block.count.value, block.bits.value, block.read_bits_per_cycle.value), (blocks, 512 * 72, 72), "{}", p.part);
        assert!(block.preloaded.value, "{}: block RAM takes its contents from the bitstream", p.part);
        let lut = of(p, MemoryKind::LutRam);
        assert_eq!((lut.count.value, lut.bits.value), (lutram, 64), "{}", p.part);
        assert!(p.memory(MemoryKind::Ultra).is_none(), "{}: a 7-series part has no UltraRAM", p.part);
    }

    let ledger: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode_images.json"))
            .expect("the decode image ledger"),
    )
    .expect("it parses");
    let images = ledger["images"].as_array().expect("images");
    let mut seen = Vec::new();
    for image in images {
        let device = image["part"].as_str().expect("a part");
        let p = PartResources::catalog()
            .into_iter()
            .find(|p| device.starts_with(p.part))
            .unwrap_or_else(|| panic!("no part for the routed device {device}"));
        let site = |s: &str| image["utilization"][s]["of"].as_u64().unwrap_or_else(|| panic!("{device}: no {s} count"));
        let block = of(&p, MemoryKind::Block36).count.value;
        assert_eq!(site("SLICE_LUTX"), 2 * p.logic.value, "{device}: two LUT sites per LUT position");
        assert_eq!(site("SLICE_FFX"), p.flops.value, "{device}: flip-flops");
        assert_eq!(site("CARRY4") * 4, p.logic.value, "{device}: four LUTs a slice");
        assert_eq!(site("RAMB36E1_RAMB36E1"), block, "{device}: 36 Kb blocks");
        assert_eq!(site("RAMB18E1_RAMB18E1"), 2 * block, "{device}: two 18 Kb halves a block");
        assert_eq!(site("DSP48E1_DSP48E1"), p.dsp.value, "{device}: DSP slices");
        seen.push(p.part);
    }
    seen.sort();
    assert_eq!(seen, vec!["xc7a100t", "xc7a200t"], "both Artix parts were checked against a routed device");
}

#[test]
fn the_artix_fabric_presets_hold_their_parts_block_ram() {
    for (target, part) in
        [(SpatialTarget::xc7a100t_fabric(1, 64), PartResources::xc7a100t()), (SpatialTarget::xc7a200t_fabric(1, 64), PartResources::xc7a200t())]
    {
        let block = of(&part, MemoryKind::Block36);
        assert_eq!(
            target.site.local_mem_bytes as u64,
            block.count.value * 4096,
            "{}: the preset's local memory is the part's {} block RAMs of 4 KiB data",
            target.name,
            block.count.value
        );
    }
}

/// DS891's Table 5 for the ZU7EV, and UG573's block shapes: a block RAM reads 72 bits a cycle and the bitstream
/// fills it; an UltraRAM is 4K x 72 on two ports and powers up zero, so nothing but a write puts a weight in it.
#[test]
fn the_zu7ev_is_ds891s_part() {
    let p = PartResources::xczu7ev();
    assert_eq!((p.logic.value, p.flops.value, p.dsp.value), (230_400, 460_800, 1_728));
    assert_eq!(p.multiplier.value, (27, 18), "DSP48E2");
    let block = of(&p, MemoryKind::Block36);
    assert_eq!((block.count.value, block.bits.value, block.read_bits_per_cycle.value), (312, 36_864, 72));
    assert!(block.preloaded.value);
    let ultra = of(&p, MemoryKind::Ultra);
    assert_eq!((ultra.count.value, ultra.bits.value, ultra.read_bits_per_cycle.value), (96, 4096 * 72, 2 * 72));
    assert!(!ultra.preloaded.value, "UltraRAM has no INIT: its contents start at zero");
    // DS891 prints "Distributed RAM (Mb) 6.2": the count is the largest LUT count that rounds to it, so a capacity
    // refused against it is refused against every part the table could describe.
    let lut = of(&p, MemoryKind::LutRam);
    let mb = |luts: u64| (luts * 64) as f64 / (1u64 << 20) as f64;
    assert!(mb(lut.count.value) <= 6.25 && mb(lut.count.value + 1) > 6.25, "{} LUTs is the top of 6.2 Mb", lut.count.value);
    assert!(mb(lut.count.value) >= 6.15);
    assert_eq!(p.block_bytes(), (312 * 36_864 + 96 * 294_912) / 8, "every block RAM and UltraRAM bit");
    assert_eq!(p.block_read_bytes_per_cycle(), (312 * 72 + 96 * 144) / 8, "every read port a cycle");
    assert!(!MemoryKind::LutRam.is_block() && MemoryKind::Block36.is_block() && MemoryKind::Ultra.is_block());
}

/// The Stratix 10 SX 2800 of the Intel PAC D5005, from the device overview's Tables 4 and 10 and the product table's
/// SX 2800 column: four registers an ALM, two 18x19 multipliers a DSP block, an M20K 512 x 40 at its widest and an
/// MLAB 32 x 20. Table 4 rounds both memories to whole binary megabits.
#[test]
fn the_sx2800_is_the_stratix_10_tables_part() {
    let p = PartResources::sx2800();
    assert_eq!(p.logic_unit, LogicUnit::Alm);
    assert_eq!((p.logic.value, p.flops.value, p.dsp.value), (933_120, 3_732_480, 5_760));
    assert_eq!(p.flops.value, 4 * p.logic.value, "four registers an ALM");
    assert_eq!(p.multiplier.value, (18, 19), "the array multiplies on a DSP block's two 18x19 halves");
    let megabits = |m: &logicaffeine_compile::spatial::part::Memory| (m.count.value * m.bits.value) as f64 / (1u64 << 20) as f64;
    let m20k = of(&p, MemoryKind::M20k);
    assert_eq!((m20k.count.value, m20k.bits.value, m20k.read_bits_per_cycle.value), (11_721, 20_480, 40));
    assert_eq!(megabits(m20k).round(), 229.0, "Table 4: \"M20K Mbits 229\"");
    assert!(m20k.preloaded.value, "an M20K runs as a ROM");
    let mlab = of(&p, MemoryKind::Mlab);
    assert_eq!((mlab.count.value, mlab.bits.value, mlab.read_bits_per_cycle.value), (23_796, 640, 20));
    assert_eq!(megabits(mlab).round(), 15.0, "Table 4: \"MLAB Mbits 15\"");
    assert!(mlab.preloaded.value, "an MLAB runs as a ROM");
    for kind in [MemoryKind::LutRam, MemoryKind::Block36, MemoryKind::Ultra] {
        assert!(p.memory(kind).is_none(), "a Stratix 10 has no {kind:?}");
    }
    assert!(MemoryKind::M20k.is_block() && !MemoryKind::Mlab.is_block(), "an MLAB is logic run as memory");
    assert_eq!(p.block_bytes(), 11_721 * 20_480 / 8, "the M20Ks alone");
    assert_eq!(p.block_read_bytes_per_cycle(), 11_721 * 40 / 8, "every M20K's widest port a cycle");
    let card = PartResources::catalog().into_iter().find(|q| "1sx280hn2f43e2vg".starts_with(q.part));
    assert_eq!(card.as_ref(), Some(&p), "the D5005's ordering code names the part");
}
