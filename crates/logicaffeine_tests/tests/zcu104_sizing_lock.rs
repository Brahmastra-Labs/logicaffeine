//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! zcu104_sizing_lock — what a token of each checkpoint we have costs on the ZCU104, and where its bytes live.
//!
//! The board is facts with sources: the XCZU7EV's block RAMs and UltraRAMs from its data sheet, the DDR4 from
//! the board guide, and what the programmable logic reads of that DDR at three operating points, the slowest
//! two measured by the FPT'19 study on the ZCU102 (the same processing system, the same DDR4 rate and width).
//! The checkpoints are their own GGUF headers, held to their files byte for byte, and every one we have ties
//! its head to its embedding table, so the lock first holds the pricer to where a tied head's bytes stream
//! from. Then it holds the committed sizing to what the pricer computes, each floor to bytes over bandwidth,
//! and the headline to the ledger: the biggest checkpoint we have that the board holds, the fastest, and the
//! one it refuses.
//!
//!   cargo nextest run -p logicaffeine-tests --test zcu104_sizing_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/cited.rs"]
mod cited;
#[path = "common/ledger_drift.rs"]
mod ledger_drift;

use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::decode::{
    price, Bind, Chip, DecodeStep, Deployment, GgufHeader, HeldClass, Machine, StageKind, Tier, TierKind,
};
use logicaffeine_compile::spatial::fpga_decode::{
    ddr_capacity, ddr_peak, zcu104, zcu104_at, zcu104_points, zcu104_sizing, Fit, ModelSizing, Zcu104Sizing,
    HP_PORT_BYTES, ZCU104_CTX, ZCU104_MODELS,
};
use logicaffeine_compile::spatial::part::{MemoryKind, PartResources};
use logicaffeine_compile::spatial::rdu_chip::Fact;

const TIED: [&str; 3] = ["qwen3.5-0.8b-q8_0", "gemma-3-1b-it-q8_0", "llama-3.2-1b-instruct-q8_0"];

fn decode_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode")
}

fn header(name: &str) -> GgufHeader {
    let path = decode_dir().join(format!("headers/{name}.json"));
    GgufHeader::read_ledger(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn bytes(h: &GgufHeader, keep: impl Fn(&str) -> bool) -> u64 {
    h.tensors
        .iter()
        .filter(|t| keep(&t.name))
        .map(|t| h.tensor_bytes(t).unwrap_or_else(|e| panic!("{}: {e:?}", t.name)))
        .sum()
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs()
}

fn regenerated() -> Zcu104Sizing {
    zcu104_sizing(&decode_dir()).expect("the sizing regenerates")
}

// ── The checkpoints, and where a tied head reads from ──────────────────────────────────────────────

/// A tied checkpoint stores its vocabulary once, as `token_embd.weight`, and reads all of it at the head. So a
/// token moves every projection once, one embedding row and the whole table, and the table is held once: a
/// pricer that also held it under the weights would need its bytes twice, and one that charged its read to the
/// weights would stream it from wherever the weights live.
#[test]
fn each_local_checkpoint_is_priced_from_its_own_header() {
    for name in TIED {
        let h = header(name);
        h.byte_audit().unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!h.tensors.iter().any(|t| t.name == "output.weight"), "{name}: a local checkpoint that ties its head");
        let blocks = h.meta_u64("block_count").expect("block_count");
        let drafts = h.meta_u64("nextn_predict_layers").unwrap_or(0);
        let draft = |n: &str| (blocks - drafts..blocks).any(|b| n.starts_with(&format!("blk.{b}.")));
        let (total, mtp, table) = (bytes(&h, |_| true), bytes(&h, draft), bytes(&h, |n| n == "token_embd.weight"));
        let vocab = h.tensors.iter().find(|t| t.name == "token_embd.weight").expect("an embedding").dims[1];
        let row = table / vocab;

        let step = DecodeStep::of(&h, &Deployment::as_stored(ZCU104_CTX)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            step.weight_bytes(),
            (total - mtp + row) as f64,
            "{name}: every projection once, the whole table at the head, one embedding row, no draft block"
        );
        let held = |c: HeldClass| step.held.iter().filter(|x| x.class == c).map(|x| x.bytes).sum::<f64>();
        let moved = |c: HeldClass| step.held.iter().filter(|x| x.class == c).map(|x| x.moved_per_token).sum::<f64>();
        assert_eq!(held(HeldClass::EmbeddingTable), table as f64, "{name}: the table is held once, as itself");
        assert_eq!(
            held(HeldClass::Weights) + held(HeldClass::EmbeddingTable),
            (total - mtp) as f64,
            "{name}: every weight the greedy step can read is held once"
        );
        let embed: f64 = step.stages.iter().filter(|s| s.kind == StageKind::Embed).map(|s| s.weight_bytes).sum();
        assert_eq!(moved(HeldClass::EmbeddingTable), embed + table as f64, "{name}: the table moves its row and the head's read");
        assert_eq!(
            moved(HeldClass::Weights),
            step.weight_bytes() - moved(HeldClass::EmbeddingTable),
            "{name}: the weights move everything else a token reads"
        );
    }
}

/// RoPE's frequency table is a weight every attention block rotates by, so it is held with the weights and read
/// once a token, by the first attention block, wherever the architecture puts that block; a table with no
/// attention block to rotate is a checkpoint the pricer cannot read, refused by the tensor's name.
#[test]
fn a_rotation_table_is_held_as_a_weight_and_read_by_the_first_attention_block() {
    let rope_in = |step: &DecodeStep| -> Vec<(Option<u32>, StageKind)> {
        step.stages.iter().filter(|s| s.tensors.iter().any(|t| t == "rope_freqs.weight")).map(|s| (s.layer, s.kind)).collect()
    };
    let llama = header("llama-3.2-1b-instruct-q8_0");
    let rope = bytes(&llama, |n| n == "rope_freqs.weight");
    assert!(rope > 0, "Llama-3.2-1B carries a rotation table");
    let step = DecodeStep::of(&llama, &Deployment::as_stored(ZCU104_CTX)).unwrap();
    assert_eq!(rope_in(&step), vec![(Some(0), StageKind::Attention)], "read once, by block 0's attention");
    let first = step.stages.iter().find(|s| s.kind == StageKind::Attention).unwrap();
    assert_eq!(first.weight_class, HeldClass::Weights);

    // A hybrid stack's first attention block is not its first block: Qwen3.5 attends every fourth.
    let mut hybrid = header("qwen3.5-0.8b-q8_0");
    let mut table = llama.tensors.iter().find(|t| t.name == "rope_freqs.weight").unwrap().clone();
    table.offset = 0;
    hybrid.tensors.push(table);
    let step = DecodeStep::of(&hybrid, &Deployment::as_stored(ZCU104_CTX)).unwrap();
    let first_attention = step.stages.iter().find(|s| s.kind == StageKind::Attention).and_then(|s| s.layer).unwrap();
    assert!(first_attention > 0, "the hybrid's first block is a recurrence");
    assert_eq!(rope_in(&step), vec![(Some(first_attention), StageKind::Attention)]);
    let held: f64 = step.held.iter().filter(|h| h.class == HeldClass::Weights).map(|h| h.bytes).sum();
    let without = DecodeStep::of(&header("qwen3.5-0.8b-q8_0"), &Deployment::as_stored(ZCU104_CTX)).unwrap();
    let held_without: f64 = without.held.iter().filter(|h| h.class == HeldClass::Weights).map(|h| h.bytes).sum();
    assert_eq!(held - held_without, rope as f64, "the table is held with the weights");

    let attends = |h: &GgufHeader, b: u64| {
        let prefix = format!("blk.{b}.");
        let block: Vec<&str> = h.tensors.iter().map(|t| t.name.as_str()).filter(|n| n.starts_with(&prefix)).collect();
        !block.is_empty() && !block.iter().any(|n| n[prefix.len()..].starts_with("ssm_"))
    };
    let blocks = hybrid.meta_u64("block_count").unwrap();
    let attending: Vec<u64> = (0..blocks).filter(|&b| attends(&hybrid, b)).collect();
    assert!(!attending.is_empty());
    let mut recurrent_only = hybrid.clone();
    recurrent_only.tensors.retain(|t| !attending.iter().any(|b| t.name.starts_with(&format!("blk.{b}."))));
    match DecodeStep::of(&recurrent_only, &Deployment::as_stored(ZCU104_CTX)) {
        Err(logicaffeine_compile::spatial::decode::DecodeRefusal::Unreadable(why)) => {
            assert!(why.contains("rope_freqs.weight"), "the refusal names the tensor: {why}")
        }
        other => panic!("a rotation table with no attention block to rotate must be refused, got {other:?}"),
    }
}

/// Residency fills the fastest tier with what a token moves the most of per byte held. Sized to hold exactly the
/// KV cache and the embedding table, a test SRAM keeps both, and the projections stream from DDR: the floor is the
/// projections over DDR's bandwidth plus the cache, the embedding row and the tied head's read of the whole table
/// over SRAM's. A pricer that charged the head to the weights would put the projections first and the table in DDR.
#[test]
fn a_tied_head_reads_the_embedding_table_from_the_tier_that_holds_it() {
    let step = DecodeStep::of(&header("llama-3.2-1b-instruct-q8_0"), &Deployment::as_stored(ZCU104_CTX)).unwrap();
    let held = |c: HeldClass| step.held.iter().filter(|h| h.class == c).map(|h| h.bytes).sum::<f64>();
    let (sram_bw, ddr_bw) = (10e12, 100e9);
    let mut chip = Chip::rtx3060();
    chip.tiers = vec![
        Tier {
            kind: TierKind::Sram,
            capacity: Fact::modeled(held(HeldClass::KvCache) + held(HeldClass::EmbeddingTable), "a test SRAM: the cache and the table"),
            bandwidth: Fact::modeled(sram_bw, "a test SRAM"),
        },
        Tier { kind: TierKind::Ddr, capacity: Fact::modeled(1e12, "a test DDR"), bandwidth: Fact::modeled(ddr_bw, "a test DDR") },
    ];
    chip.matrix = vec![];
    chip.stage_overhead = None;
    chip.link = None;
    let cost = price(&step, &Machine::of(chip, 1), Precision::Int8).unwrap();
    let of = |k: StageKind| step.stages.iter().filter(|s| s.kind == k).map(|s| s.weight_bytes).sum::<f64>();
    let (head, embed) = (of(StageKind::Head), of(StageKind::Embed));
    assert!(close(head, held(HeldClass::EmbeddingTable), 1e-12), "the tied head reads the whole table");
    let state = step.state_read_bytes() + step.state_write_bytes();
    let projections = step.weight_bytes() - head - embed;
    let want = projections / ddr_bw + (head + embed + state) / sram_bw;
    assert!(
        close(cost.floor_s, want, 1e-9),
        "floor {} vs {want}: the projections stream from DDR, the cache, the row and the tied head from SRAM",
        cost.floor_s
    );
}

// ── The board ──────────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_zcu104_fact_names_its_source() {
    for p in zcu104_points() {
        let chip = zcu104_at(&p);
        cited::assert_cited(chip.name, &chip.facts(), chip.sources);
        for tier in &chip.tiers {
            assert!(tier.capacity.value > 0.0 && tier.bandwidth.value > 0.0, "{} {:?}", chip.name, tier.kind);
        }
        let point = [("pl_hz", p.pl_hz.provenance, p.pl_hz.cite), ("ddr_read", p.ddr_read.provenance, p.ddr_read.cite)];
        let point: Vec<(String, _, _)> = point.into_iter().map(|(n, v, c)| (n.to_string(), v, c)).collect();
        cited::assert_cited(p.name, &point, chip.sources);
    }
    assert_eq!(Chip::by_name("zcu104"), Some(zcu104()), "the catalog prices the ZCU104 at its four-port, 100 MHz point");
}

#[test]
fn the_zcu104_holds_its_parts_on_chip_ram_and_its_ps_ddr() {
    let part = PartResources::xczu7ev();
    let block = part.memory(MemoryKind::Block36).expect("block RAM");
    let ultra = part.memory(MemoryKind::Ultra).expect("UltraRAM");
    assert_eq!((block.count.value, ultra.count.value), (312, 96), "DS891 Table 5: ZU7EV");
    for p in zcu104_points() {
        let chip = zcu104_at(&p);
        let [sram, ddr] = chip.tiers.as_slice() else { panic!("{}: an SRAM tier and a DDR tier", chip.name) };
        assert_eq!((sram.kind, ddr.kind), (TierKind::Sram, TierKind::Ddr), "{}: fastest first", chip.name);
        assert_eq!(sram.capacity.value, (312.0 * 512.0 * 72.0 + 96.0 * 4096.0 * 72.0) / 8.0, "{}: every block's bits", chip.name);
        assert_eq!(sram.capacity.value, part.block_bytes() as f64);
        assert_eq!(ddr.capacity.value, 2f64.powi(31), "{}: four 4 Gb DDR4 parts", chip.name);
        assert!(
            close(sram.bandwidth.value, (312.0 * 72.0 + 96.0 * 2.0 * 72.0) / 8.0 * p.pl_hz.value, 1e-12),
            "{}: every block RAM and UltraRAM port read each PL cycle",
            chip.name
        );
        let int8 = chip.matrix_rate(Precision::Int8).expect("an int8 rate");
        assert!(close(int8.value, 1728.0 * 2.0 * 2.0 * p.pl_hz.value, 1e-12), "{}: two int8 MACs per DSP slice per cycle", chip.name);
        assert_eq!(int8.provenance, p.pl_hz.provenance, "{}: a rate read off the clock is as strong as the clock", chip.name);
    }
    assert_eq!(ddr_capacity().value, 2_147_483_648.0);
}

#[test]
fn the_pl_reads_the_ddr_at_its_ports_times_its_clock_up_to_the_ddrs_own_rate() {
    let peak = ddr_peak().value;
    assert_eq!(peak, 2400e6 * 8.0, "DDR4-2400 on a 64-bit bus");
    let points = zcu104_points();
    for p in &points {
        let ports = p.hp_ports as f64 * HP_PORT_BYTES * p.pl_hz.value;
        assert!(
            p.ddr_read.value <= ports.min(peak) * (1.0 + 1e-12),
            "{}: {} B/s read through {} ports at {} Hz, which carry at most {}",
            p.name,
            p.ddr_read.value,
            p.hp_ports,
            p.pl_hz.value,
            ports.min(peak)
        );
    }
    assert_eq!(points[0].ddr_read.value, 4.0 * HP_PORT_BYTES * 100e6, "at 100 MHz all four ports run at their maximum");
    let last = points.last().expect("a peak point");
    assert_eq!(last.ddr_read.value, peak, "the last point is the DDR's own rate");
    assert_eq!(last.hp_ports as f64 * HP_PORT_BYTES * last.pl_hz.value, peak, "at the slowest clock that reaches it");
    assert!(points.windows(2).all(|w| w[0].ddr_read.value < w[1].ddr_read.value), "slowest first");
}

// ── The sizing ─────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_committed_sizing_is_what_the_planner_computes() {
    let committed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(decode_dir().join("zcu104_sizing.json")).expect("the committed sizing"),
    )
    .expect("it parses");
    let fresh = serde_json::to_value(regenerated()).expect("it serializes");
    if let Some(path) = ledger_drift::first_difference(&committed, &fresh, String::new()) {
        panic!(
            "benchmarks/silicon/decode/zcu104_sizing.json is stale at `{path}`: regenerate it with `cargo run -p \
             logicaffeine-compile --example zcu104_sizing`"
        );
    }
}

/// The board's on-chip RAM moves at most twice its bytes a token (a recurrent state, read and written), so a
/// checkpoint far past it reads nearly all of itself from DDR: each floor sits between every byte over DDR's
/// bandwidth and every byte the on-chip RAM cannot take over it, binds on memory, and rises with the point's DDR read.
#[test]
fn every_floor_is_its_bytes_over_its_bandwidth() {
    let sizing = regenerated();
    let points = zcu104_points();
    let mut rows = 0;
    for m in &sizing.models {
        let Fit::Resident { rows: priced } = &m.fit else { continue };
        let step = DecodeStep::of(&header(&m.model), &Deployment::as_stored(sizing.ctx)).unwrap();
        let total = m.weight_bytes_per_token + m.state_bytes_per_token;
        assert_eq!(total, step.weight_bytes() + step.state_read_bytes() + step.state_write_bytes(), "{}", m.model);
        assert_eq!(priced.len(), points.len(), "{}: a row at every point", m.model);
        for (row, p) in priced.iter().zip(&points) {
            assert_eq!(row.point, p.name);
            let cost = price(&step, &Machine::of(zcu104_at(p), 1), Precision::Int8).unwrap();
            assert_eq!(row.floor_tok_s, cost.floor_tokens_per_s(), "{} at {}: the row is the price", m.model, p.name);
            assert_eq!(row.binds, Bind::Memory, "{} at {}: batch-1 decode binds on memory", m.model, p.name);
            let ddr = p.ddr_read.value;
            assert!(row.floor_tok_s >= ddr / total, "{} at {}: no slower than every byte from DDR", m.model, p.name);
            assert!(
                row.floor_tok_s <= ddr / (total - 2.0 * sizing.on_chip_bytes),
                "{} at {}: {} tok/s, faster than the on-chip RAM can make it",
                m.model,
                p.name,
                row.floor_tok_s
            );
            rows += 1;
        }
        assert!(priced.windows(2).all(|w| w[0].floor_tok_s < w[1].floor_tok_s), "{}: faster as the DDR read rises", m.model);
    }
    assert!(rows >= 9, "{rows} rows checked");
}

#[test]
fn a_checkpoint_past_the_boards_two_gigabytes_is_refused_with_its_size() {
    let sizing = regenerated();
    let m = sizing.models.iter().find(|m| m.model == "qwen3-8b-q8_0").expect("qwen3-8b is sized");
    match &m.fit {
        Fit::NotResident { needed, capacity } => {
            assert_eq!(*needed, m.held_bytes);
            assert_eq!(*capacity, sizing.on_chip_bytes + sizing.ddr_bytes, "the board's whole memory");
            assert!(needed > capacity);
        }
        other => panic!("an 8.7 GB checkpoint does not fit 2 GB, got {other:?}"),
    }
}

/// The question the sizing answers, held to the ledger it answers from: of the checkpoints we have, the biggest the
/// board holds is Llama-3.2-1B, the fastest is Qwen3.5-0.8B at every point, and every one it holds is tied.
#[test]
fn the_biggest_checkpoint_we_have_that_fits_is_llama_3_2_1b_and_the_fastest_is_qwen3_5_0_8b() {
    let sizing = regenerated();
    assert_eq!(sizing.models.iter().map(|m| m.model.as_str()).collect::<Vec<_>>(), ZCU104_MODELS);
    let resident: Vec<_> = sizing.models.iter().filter(|m| matches!(m.fit, Fit::Resident { .. })).collect();
    assert_eq!(resident.len(), 3, "three local checkpoints fit the board");
    let biggest = resident.iter().max_by(|a, b| a.held_bytes.total_cmp(&b.held_bytes)).unwrap();
    assert_eq!(biggest.model, "llama-3.2-1b-instruct-q8_0");
    assert!(resident.iter().all(|m| m.tied_head), "every checkpoint the board holds ties its head");
    for (i, p) in zcu104_points().iter().enumerate() {
        let rate = |m: &ModelSizing| match &m.fit {
            Fit::Resident { rows } => rows[i].floor_tok_s,
            Fit::NotResident { .. } => unreachable!(),
        };
        let fastest = resident.iter().max_by(|a, b| rate(a).total_cmp(&rate(b))).unwrap();
        assert_eq!(fastest.model, "qwen3.5-0.8b-q8_0", "at {}", p.name);
    }
}
