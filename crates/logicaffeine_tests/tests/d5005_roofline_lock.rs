//! The D5005 roofline: Qwen3.8-27B priced on the Intel PAC D5005 at every knob point of the decode engine.
//!
//! `benchmarks/silicon/d5005/roofline.json` is what the serving pricer computes from the checkpoints' own headers
//! and the card's cited facts. These tests hold the ledger to the pricer, the pricer to the batch-one step it is
//! read from, the card's stream shares to the DDR4 ledger, and the engine's tricks to what they claim: no trick costs
//! throughput and each buys some, a pass pays the weights once and each sequence its own bytes, each at the share of
//! the way it moves, every rung runs at its fastest batch and memory split, and the DSP packing keeps its products
//! apart.

use std::path::{Path, PathBuf};

use logicaffeine_compile::spatial::d5005::{
    self, card, chip, ladder, measured_acceptance, mixed, roofline, Beat, EnginePlan, StreamClocks, ACCEPTANCE_POINTER, ALMS,
    ARRAY_MACS_PER_DSP, CONTEXT_SWEEP, DDR_STREAMS, DSP_BLOCKS, LANE_MIX, M20K_BITS, M20K_BLOCKS, SCALE_LANE_DSPS,
};
use logicaffeine_compile::spatial::decode_engine::ddr::{CACHE_POSITIONS, STATE_CHUNKS};
use logicaffeine_compile::spatial::decode_engine::numerics::ActScaling;
use logicaffeine_compile::spatial::part::{MemoryKind, PartResources};
use logicaffeine_compile::spatial::decode::{DecodeStep, Deployment, GgmlType, GgufHeader, WeightBytes};
use logicaffeine_compile::spatial::rdu_chip::Provenance;
use logicaffeine_compile::spatial::serving::{
    array_passes, emitted_per_pass, lane_cycles, max_batch, price_serving, AttentionMath, ContextClass, ContextMix, Draft,
    PassShape, Serving, ServingBind, ServingRefusal, StateFlush, LANE_PRODUCTS, SOFTMAX_MACS_PER_SCORE, VALUE_BLOCK,
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn decode_dir() -> PathBuf {
    repo().join("benchmarks/silicon/decode")
}

fn header(name: &str) -> GgufHeader {
    let path = decode_dir().join(format!("headers/{name}.json"));
    GgufHeader::read_ledger(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * a.abs().max(b.abs())
}

/// A greedy serving at f16 state and int8 KV, the shape most tests probe.
fn plain(batch: u32) -> Serving {
    Serving {
        batch,
        contexts: ContextMix::uniform(1024.0, 2048),
        kv_page: None,
        attention: AttentionMath::Vector,
        kv_bytes: 1.0,
        state_bytes: 2.0,
        activation_words: 1,
        scaling: ActScaling::PerBlock,
        kv_rotated: false,
        draft: None,
        flush: StateFlush::EveryPass,
        host_memory: false,
    }
}

// ── The ledger is the pricer's ─────────────────────────────────────────────────────────────────────

fn first_difference(a: &serde_json::Value, b: &serde_json::Value, at: String) -> Option<String> {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            let (x, y) = (x.as_f64()?, y.as_f64()?);
            (!close(x, y, 1e-9)).then_some(format!("{at}: {x} vs {y}"))
        }
        (Array(x), Array(y)) if x.len() == y.len() => {
            x.iter().zip(y).enumerate().find_map(|(i, (x, y))| first_difference(x, y, format!("{at}/{i}")))
        }
        (Object(x), Object(y)) if x.len() == y.len() && x.keys().all(|k| y.contains_key(k)) => {
            x.iter().find_map(|(k, v)| first_difference(v, &y[k], format!("{at}/{k}")))
        }
        _ => (a != b).then_some(format!("{at}: {a} vs {b}")),
    }
}

#[test]
fn the_committed_roofline_is_what_the_pricer_computes() {
    let path = repo().join("benchmarks/silicon/d5005/roofline.json");
    let committed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
            .expect("the committed roofline parses");
    let fresh = serde_json::to_value(roofline(&decode_dir()).expect("the roofline prices")).expect("serializes");
    if let Some(at) = first_difference(&committed, &fresh, String::new()) {
        panic!(
            "benchmarks/silicon/d5005/roofline.json is stale at `{at}`: regenerate it with `cargo run -p \
             logicaffeine-compile --example d5005_roofline`"
        );
    }
}

#[test]
fn every_row_names_the_term_that_binds_it() {
    let r = roofline(&decode_dir()).unwrap();
    assert!(r.rows.len() >= 20, "{} rows", r.rows.len());
    for row in &r.rows {
        let c = &row.cost;
        let want = match c.binds {
            ServingBind::Memory => c.memory_s,
            ServingBind::Host => c.host_s,
            ServingBind::Array => c.array_s,
            ServingBind::Vector => c.vector_s,
            ServingBind::Scale => c.scale_s,
        };
        assert_eq!(c.pass_s, want, "{}: binds {:?}", row.label, c.binds);
        assert!(
            c.pass_s >= c.memory_s && c.pass_s >= c.host_s && c.pass_s >= c.array_s && c.pass_s >= c.vector_s && c.pass_s >= c.scale_s,
            "{}",
            row.label
        );
        assert!(close(row.tokens_per_s, c.batch as f64 * c.emitted_per_sequence / c.pass_s, 1e-12), "{}", row.label);
    }
}

// ── The facts ──────────────────────────────────────────────────────────────────────────────────────

fn assert_cited(name: &str, facts: &[(String, Provenance, &'static str)], sources: &[&str]) {
    assert!(!facts.is_empty(), "{name}: no facts");
    for (path, provenance, cite) in facts {
        assert!(!cite.trim().is_empty(), "{name} `{path}`: an empty citation");
        match provenance {
            Provenance::Published | Provenance::Derived => assert!(
                sources.iter().any(|s| cite.contains(s)),
                "{name} `{path}` is {provenance:?} but cites `{cite}`, which names none of its sources {sources:?}"
            ),
            Provenance::Measured => assert!(
                cite.contains("benchmarks/") && cite.contains(".json"),
                "{name} `{path}` is measured but cites `{cite}`, which names no run ledger"
            ),
            Provenance::Modeled => assert!(
                ["not public", "not published", "inherited", "not tabulated"].iter().any(|m| cite.contains(m)),
                "{name} `{path}` is modeled but its cite `{cite}` does not say what gap it fills"
            ),
        }
    }
}

#[test]
fn every_card_fact_names_its_source() {
    let plan = EnginePlan::standard();
    let c = card(&plan);
    assert_cited("d5005 card", &c.facts(), c.sources);
    let ch = chip();
    assert_cited("d5005 chip", &ch.facts(), ch.sources);
    assert_cited(
        "d5005 plan",
        &[
            ("clock_hz".into(), plan.clock_hz.provenance, plan.clock_hz.cite),
            ("array_soft_macs".into(), plan.array_soft_macs.provenance, plan.array_soft_macs.cite),
        ],
        d5005::D5005_SOURCES,
    );
}

#[test]
fn the_card_is_its_data_sheet() {
    let c = card(&EnginePlan::standard());
    assert_eq!(c.memory.capacity.value, 4.0 * 8.0 * (1u64 << 30) as f64, "four channels of 8 GiB");
    assert_eq!(c.memory.bandwidth.value, 4.0 * 2400e6 * 8.0, "four channels of DDR4-2400, 8 data bytes a transfer");
    assert_eq!(c.on_chip.capacity.value, 11_721.0 * 20_480.0 / 8.0, "11,721 M20K blocks of 20 Kb");
    assert_eq!((DSP_BLOCKS, ALMS), (5_760, 933_120));
}

/// The card's streams are the committed DDR4 ledger's, row for row in the ledger's order — each beat's weights, its
/// state at every chunk, its cache at every context — clock for clock; and a plan reads its beat's weights row, its
/// chunk's state row and the least of its cache rows.
#[test]
fn the_cards_stream_shares_are_the_ddr_ledgers() {
    let path = repo().join("benchmarks/silicon/d5005/ddr4_efficiency.json");
    let ledger: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rows = ledger["rows"].as_array().expect("the ledger's rows");
    // Each mirrored stream with the phrases that name its row.
    let mut mirrored: Vec<(Beat, Vec<String>, StreamClocks)> = Vec::new();
    for b in &DDR_STREAMS {
        mirrored.push((b.beat, vec!["weights: one read".into()], b.weights));
        for (chunk, c) in STATE_CHUNKS.iter().zip(b.state) {
            mirrored.push((b.beat, vec!["Gated-DeltaNet state:".into(), format!("read and written back {chunk} lines at a time")], c));
        }
        for (positions, c) in CACHE_POSITIONS.iter().zip(b.cache) {
            mirrored.push((b.beat, vec![format!("attention cache: 4 sequences of {positions} positions read, one appended")], c));
        }
    }
    assert_eq!(rows.len(), mirrored.len(), "the ledger's rows against the mirror's streams");
    for (row, (beat, phrases, clocks)) in rows.iter().zip(&mirrored) {
        let pattern = row["pattern"].as_str().expect("a row's pattern");
        assert_eq!(row["beat"], serde_json::to_value(beat).unwrap(), "`{pattern}`");
        for p in phrases {
            assert!(pattern.contains(p.as_str()), "the ledger's `{pattern}` is mirrored as `{p}`");
        }
        assert_eq!(row["facts"]["data_ck"].as_u64(), Some(clocks.data_ck), "`{pattern}`: data clocks");
        assert_eq!(row["facts"]["end"].as_u64(), Some(clocks.end), "`{pattern}`: clocks");
        // serde_json reads a decimal to within an ulp of the binary64 it was printed from.
        let share = row["efficiency"].as_f64().expect("a row's share");
        assert!((share - clocks.share()).abs() <= f64::EPSILON * clocks.share(), "`{pattern}`: share {share} vs {}", clocks.share());
    }
    for plan in [EnginePlan::standard(), EnginePlan::full_beat()] {
        let s = card(&plan).streams;
        let b = DDR_STREAMS.iter().find(|b| b.beat == plan.beat).unwrap();
        let chunk = STATE_CHUNKS.iter().position(|&c| c == plan.state_chunk_lines).expect("the plan's chunk is the ledger's");
        assert_eq!(s.read.value, b.weights.share(), "{:?}", plan.beat);
        assert_eq!(s.rewrite.value, b.state[chunk].share(), "{:?}", plan.beat);
        let least = b.cache.iter().map(|c| c.share()).fold(f64::INFINITY, f64::min);
        assert_eq!(s.append.value, least, "{:?}", plan.beat);
        assert!(b.cache.iter().any(|c| c.share() == least));
    }
}

/// The standard plan rewrites a state a value head at a time: 128 by 128 binary16 elements, at 64 bytes a line.
#[test]
fn the_standard_plan_rewrites_a_state_a_value_head_at_a_time() {
    assert_eq!(EnginePlan::standard().state_chunk_lines * 64, 128 * 128 * 2);
}

/// A plan whose state chunk the ledger never priced has no stream share to read, and is refused rather than guessed.
#[test]
#[should_panic(expected = "the DDR4 ledger prices")]
fn a_state_chunk_the_ledger_never_priced_is_refused() {
    card(&EnginePlan { state_chunk_lines: 256, ..EnginePlan::standard() });
}

#[test]
fn the_plan_spends_what_the_device_has() {
    let plan = EnginePlan::standard();
    assert!(plan.array_dsp_blocks + plan.vector_dsp_blocks <= DSP_BLOCKS);
    // The modeled soft multiply-adds at their modeled 15 ALMs each stay within 40% of the device.
    assert!(plan.array_soft_macs.value * 15.0 <= 0.4 * ALMS as f64, "{} soft multiply-adds", plan.array_soft_macs.value);
    let c = card(&plan);
    let want = (plan.array_dsp_blocks as f64 * ARRAY_MACS_PER_DSP as f64 + plan.array_soft_macs.value) * plan.clock_hz.value;
    assert_eq!(c.array.value, want);
    assert_eq!(c.vector.value, plan.vector_dsp_blocks as f64 * plan.clock_hz.value);
}

#[test]
fn every_acceptance_is_the_measured_one() {
    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo().join("benchmarks/results/qwen38_27b.json")).unwrap()).unwrap();
    let measured = ledger.pointer(ACCEPTANCE_POINTER).and_then(|v| v.as_f64()).expect("the measured acceptance");
    let r = roofline(&decode_dir()).unwrap();
    let mut drafted = 0;
    for row in &r.rows {
        if let Some(d) = &row.serving.draft {
            drafted += 1;
            for a in &d.acceptance {
                assert_eq!(a.provenance, Provenance::Measured, "{}: an acceptance must be measured, never assumed", row.label);
                assert_eq!(a.value, measured, "{}", row.label);
            }
        }
    }
    assert!(drafted >= 4, "{drafted} drafted rows");
}

// ── The tricks ─────────────────────────────────────────────────────────────────────────────────────

/// Down each context's ladder no trick costs throughput, and every trick buys some on at least one context: a
/// trick that relieves a term that does not bind there buys nothing there, and one that never buys anything is
/// not a rung.
#[test]
fn no_trick_costs_throughput_and_every_trick_buys_some() {
    let r = roofline(&decode_dir()).unwrap();
    let ladders: Vec<Vec<_>> = ["2k: ", "8k: "]
        .iter()
        .map(|tag| r.rows.iter().filter(|row| row.label.starts_with(tag)).collect::<Vec<_>>())
        .collect();
    let rungs = ladders[0].len();
    assert!(rungs >= 10 && ladders.iter().all(|l| l.len() == rungs), "every context climbs the same ladder");
    for i in 1..rungs {
        let mut bought = false;
        for l in &ladders {
            let (before, after) = (l[i - 1], l[i]);
            assert!(
                after.tokens_per_s >= before.tokens_per_s * (1.0 - 1e-12),
                "`{}` ({:.1} tok/s) is slower than `{}` ({:.1} tok/s)",
                after.label,
                after.tokens_per_s,
                before.label,
                before.tokens_per_s
            );
            bought |= after.tokens_per_s > before.tokens_per_s * (1.0 + 1e-9);
        }
        assert!(bought, "`{}` buys throughput on no context", &ladders[0][i].label[4..]);
    }
    let first = ladders[0][0].tokens_per_s;
    let last = ladders[0][rungs - 1].tokens_per_s;
    assert!(last > 10.0 * first, "the whole ladder: {first:.1} → {last:.1} tok/s");
}

/// Every rung runs at the batch that emits the most tokens per second, and with host memory at the fastest split
/// of that batch: no other batch, and no other split, beats it.
#[test]
fn every_rung_runs_at_its_fastest_batch_and_split() {
    let r = roofline(&decode_dir()).unwrap();
    let c = card(&EnginePlan::standard());
    for row in &r.rows {
        let shape = PassShape::of(&header(row.header), &row.serving).unwrap();
        let chosen = row.tokens_per_s;
        let mut tried = 0;
        for batch in 1..=row.serving.batch * 2 + 8 {
            let split = match shape.cost_split(&c, batch, row.serving.host_memory) {
                Ok(cost) => cost,
                Err(ServingRefusal::NotResident { .. } | ServingRefusal::OnChipOverflow { .. }) => break,
                Err(e) => panic!("{}: {e:?}", row.label),
            };
            assert!(
                split.tokens_per_s() <= chosen * (1.0 + 1e-12),
                "{}: batch {batch} emits {:.2} tok/s, more than the chosen {} at {:.2}",
                row.label,
                split.tokens_per_s(),
                row.serving.batch,
                chosen
            );
            if batch < row.serving.batch {
                assert!(
                    split.tokens_per_s() * (1.0 + 1e-12) < chosen,
                    "{}: batch {batch} ties the chosen {}, and a tie goes to the smaller batch",
                    row.label,
                    row.serving.batch
                );
            }
            if row.serving.host_memory {
                for host in 0..=batch {
                    if let Ok(other) = shape.cost(&c, batch, host) {
                        assert!(other.pass_s >= split.pass_s * (1.0 - 1e-12), "{}: batch {batch}, {host} in host beats the chosen split", row.label);
                    }
                }
            }
            tried += 1;
        }
        assert!(tried >= row.serving.batch as usize, "{}: tried {tried}", row.label);
        assert!(row.cost.held_bytes <= row.cost.capacity_bytes && row.cost.host_held_bytes <= row.cost.host_capacity_bytes, "{}", row.label);
    }
}

#[test]
fn a_host_sequence_streams_its_bytes_over_the_link_both_ways_at_once() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let shape = PassShape::of(&h, &Serving { host_memory: true, ..plain(1) }).unwrap();
    let link = c.host.unwrap();
    for host in [1u32, 5, 20] {
        let cost = shape.cost(&c, 40, host).unwrap();
        let want = (host as f64 * shape.read_bytes / (link.read.value * link.stream.value))
            .max(host as f64 * shape.write_bytes / (link.write.value * link.stream.value));
        assert!(close(cost.host_s, want, 1e-12), "{host} host sequences: {} vs {want}", cost.host_s);
        let at = |share: f64| c.memory.bandwidth.value * share;
        let card_s = (shape.weight_bytes + 40.0 * shape.embed_bytes) / at(c.streams.read.value)
            + (40 - host) as f64
                * (shape.rewrite_bytes / at(c.streams.rewrite.value)
                    + (shape.read_bytes + shape.write_bytes - shape.rewrite_bytes) / at(c.streams.append.value));
        assert!(close(cost.memory_s, card_s, 1e-12), "{host} host sequences: the card's memory {} vs {card_s}", cost.memory_s);
    }
    let mut no_link = c.clone();
    no_link.host = None;
    match shape.cost(&no_link, 40, 1) {
        Err(ServingRefusal::NoHostLink) => {}
        other => panic!("host sequences without a host link must be refused, got {other:?}"),
    }
}

#[test]
fn paged_caches_hold_each_sequence_its_own_length() {
    let h = header("qwen3.8-27b-q4_k_m");
    let contiguous = PassShape::of(&h, &plain(1)).unwrap();
    let paged = PassShape::of(&h, &Serving { kv_page: Some(256), ..plain(1) }).unwrap();
    // 16 attention layers, 4 KV heads, a 256-element key and value each, one byte an element.
    let per_position = 16.0 * 4.0 * 512.0;
    let saved = (2048.0 - (1024.0 + 128.0)) * per_position;
    assert!(close(contiguous.held_per_sequence - paged.held_per_sequence, saved, 1e-12));
    assert_eq!(contiguous.read_bytes, paged.read_bytes, "paging changes what is held, not what a pass reads");
}

#[test]
fn a_pass_pays_the_weights_once_and_each_sequence_its_own_bytes() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let one = price_serving(&h, &c, &plain(1)).unwrap();
    let embed_row = DecodeStep::of(&h, &Deployment { ctx: 1024, weights: WeightBytes::AsStored, kv_bytes: 1.0, state_bytes: 2.0, activation_bytes: 2.0, speculative: false })
        .unwrap()
        .stages[0]
        .weight_bytes;
    for batch in [2u32, 7, 40] {
        let many = price_serving(&h, &c, &plain(batch)).unwrap();
        assert_eq!(many.sequence_bytes, one.sequence_bytes, "B={batch}: a sequence's bytes do not depend on its batch");
        assert!(close(many.weight_bytes - one.weight_bytes, (batch - 1) as f64 * embed_row, 1e-12), "B={batch}: only embedding rows grow");
        assert!(close(many.array_macs, batch as f64 * one.array_macs, 1e-12), "B={batch}: every row multiplies every weight");
    }
}

/// At one sequence with no draft, a pass reads what the batch-one step reads, but for the conv ring: the step
/// rewrites the whole ring, the pass appends one row of it.
#[test]
fn batch_one_without_drafts_moves_what_the_decode_step_moves() {
    for name in ["qwen3.8-27b-q4_k_m", "qwen3.8-27b-q8_0", "qwen3.8-27b-q3_k_m"] {
        let h = header(name);
        let s = Serving { state_bytes: 4.0, kv_bytes: 2.0, ..plain(1) };
        let step = DecodeStep::of(&h, &Deployment { ctx: 1024, weights: WeightBytes::AsStored, kv_bytes: 2.0, state_bytes: 4.0, activation_bytes: 2.0, speculative: false }).unwrap();
        let cost = price_serving(&h, &card(&EnginePlan::standard()), &s).unwrap();
        assert!(close(cost.weight_bytes, step.weight_bytes(), 1e-12), "{name}: {} vs {}", cost.weight_bytes, step.weight_bytes());
        let taps = h.meta_u64("ssm.conv_kernel").unwrap();
        let gdn_layers = step.stages.iter().filter(|st| st.kind == logicaffeine_compile::spatial::decode::StageKind::Recurrence).count() as f64;
        let channels = 2048.0 * 2.0 + 6144.0;
        let ring_rewrite_saved = gdn_layers * (taps - 2) as f64 * channels * 4.0;
        let step_bytes = step.state_read_bytes() + step.state_write_bytes();
        assert!(close(cost.sequence_bytes, step_bytes - ring_rewrite_saved, 1e-12), "{name}: {} vs {}", cost.sequence_bytes, step_bytes - ring_rewrite_saved);
    }
}

#[test]
fn a_draft_emits_between_one_token_and_every_row() {
    assert_eq!(emitted_per_pass(&[]), 1.0);
    assert_eq!(emitted_per_pass(&[1.0]), 2.0);
    assert_eq!(emitted_per_pass(&[0.5, 0.5]), 1.75);
    assert_eq!(emitted_per_pass(&[0.0, 1.0]), 1.0, "a rejected draft ends the run");
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let d = |a: f64| Serving {
        draft: Some(Draft { tokens: 1, acceptance: vec![measured_acceptance(a)], head_rows: None }),
        flush: StateFlush::Every { tokens: 16 },
        ..plain(8)
    };
    let cost = price_serving(&h, &c, &d(0.924)).unwrap();
    assert_eq!(cost.rows_per_sequence, 2);
    assert!(cost.emitted_per_sequence > 1.0 && cost.emitted_per_sequence < 2.0);
    match price_serving(&h, &c, &Serving { draft: Some(Draft { tokens: 2, acceptance: vec![measured_acceptance(0.9)], head_rows: None }), ..plain(1) }) {
        Err(ServingRefusal::BadDraft(_)) => {}
        other => panic!("two drafts with one acceptance must be refused, got {other:?}"),
    }
}

#[test]
fn a_drafting_head_over_fewer_rows_reads_fewer_bytes_and_emits_the_same_tokens() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let with = |rows: Option<u64>| Serving {
        draft: Some(Draft { tokens: 1, acceptance: vec![measured_acceptance(0.924)], head_rows: rows }),
        flush: StateFlush::Every { tokens: 16 },
        ..plain(16)
    };
    let full = price_serving(&h, &c, &with(None)).unwrap();
    let trimmed = price_serving(&h, &c, &with(Some(32_768))).unwrap();
    let head = h.tensors.iter().find(|t| t.name == "output.weight").unwrap();
    let head_bytes = h.tensor_bytes(head).unwrap() as f64;
    assert!(close(full.weight_bytes - trimmed.weight_bytes, head_bytes * (1.0 - 32_768.0 / 248_320.0), 1e-9));
    assert_eq!(full.emitted_per_sequence, trimmed.emitted_per_sequence, "verification reads the whole head");
}

#[test]
fn the_lazy_flush_reads_the_state_once_and_writes_it_once_in_a_while() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let every_pass = price_serving(&h, &c, &plain(4)).unwrap();
    let lazy = price_serving(&h, &c, &Serving { flush: StateFlush::Every { tokens: 16 }, ..plain(4) }).unwrap();
    assert!(lazy.sequence_bytes < every_pass.sequence_bytes, "{} vs {}", lazy.sequence_bytes, every_pass.sequence_bytes);
    // Flushed every token, the lazy form writes the state as often and adds its one-token log.
    let each = price_serving(&h, &c, &Serving { flush: StateFlush::Every { tokens: 1 }, ..plain(4) }).unwrap();
    let gdn_layers = 48.0;
    let log = (16.0 * 128.0 + 48.0 * 128.0 + 2.0 * 48.0) * 2.0;
    assert!(close(each.sequence_bytes, every_pass.sequence_bytes + gdn_layers * log, 1e-12), "{} vs {}", each.sequence_bytes, every_pass.sequence_bytes + gdn_layers * log);
}

/// A state kept eagerly is read and written back in place every pass, all of it rewritten; a lazily flushed one only
/// on the passes that flush it — flushed every token as much as the eager one, every 16 a sixteenth — and read alone
/// on the others. The caches, the rings and the logs are read and appended, never rewritten.
#[test]
fn a_state_is_rewritten_in_place_only_on_the_passes_that_write_it() {
    let h = header("qwen3.8-27b-q4_k_m");
    let eager = PassShape::of(&h, &plain(4)).unwrap();
    let lazy = |tokens| PassShape::of(&h, &Serving { flush: StateFlush::Every { tokens }, ..plain(4) }).unwrap();
    let (gdn_layers, state) = (48.0, 48.0 * 128.0 * 128.0 * 2.0);
    assert!(close(eager.rewrite_bytes, gdn_layers * 2.0 * state, 1e-12), "{}", eager.rewrite_bytes);
    assert!(close(lazy(1).rewrite_bytes, eager.rewrite_bytes, 1e-12), "{}", lazy(1).rewrite_bytes);
    assert!(close(lazy(16).rewrite_bytes, eager.rewrite_bytes / 16.0, 1e-12), "{}", lazy(16).rewrite_bytes);
    for s in [&eager, &lazy(1), &lazy(16)] {
        assert!(s.rewrite_bytes < s.read_bytes + s.write_bytes, "the caches and rings are rewritten");
    }
    // The cost prices each at its own share, the bytes it reads alone at the share of a read with an append.
    let c = card(&EnginePlan::standard());
    let lazy16 = lazy(16);
    let at = |share: f64| c.memory.bandwidth.value * share;
    let cost = lazy16.cost(&c, 4, 0).unwrap();
    let want = cost.weight_bytes / at(c.streams.read.value)
        + 4.0 * (lazy16.rewrite_bytes / at(c.streams.rewrite.value) + (cost.sequence_bytes - lazy16.rewrite_bytes) / at(c.streams.append.value));
    assert!(close(cost.memory_s, want, 1e-12), "{} vs {want}", cost.memory_s);
    assert_eq!(cost.rewrite_bytes, lazy16.rewrite_bytes);
}

/// A trailing flush moves what a flush every window moves but its log: every pass reads and writes back its emitted
/// tokens' shares of the state, a flush's worth over the window as the flush every window does, and reads one window
/// more of the log, which the card holds besides; at a pass's arithmetic it walks that window more.
#[test]
fn a_trailing_flush_writes_what_a_flush_every_window_writes_and_reads_a_window_more_log() {
    let h = header("qwen3.8-27b-q4_k_m");
    let shape = |flush| PassShape::of(&h, &Serving { flush, ..plain(4) }).unwrap();
    let (gdn_layers, log) = (48.0, (16.0 * 128.0 + 48.0 * 128.0 + 2.0 * 48.0) * 2.0);
    for tokens in [1u32, 8, 16] {
        let (every, trailing) = (shape(StateFlush::Every { tokens }), shape(StateFlush::Trailing { tokens }));
        let window = gdn_layers * tokens as f64 * log;
        assert!(close(trailing.write_bytes, every.write_bytes, 1e-12), "{tokens}: writes {} vs {}", trailing.write_bytes, every.write_bytes);
        assert!(close(trailing.rewrite_bytes, every.rewrite_bytes, 1e-12), "{tokens}: rewritten {} vs {}", trailing.rewrite_bytes, every.rewrite_bytes);
        assert!(close(trailing.read_bytes, every.read_bytes + window, 1e-12), "{tokens}: reads {} vs {} and a window", trailing.read_bytes, every.read_bytes);
        assert!(close(trailing.held_per_sequence, every.held_per_sequence + window, 1e-12), "{tokens}: held {} vs {} and a window", trailing.held_per_sequence, every.held_per_sequence);
        assert!(trailing.vector_macs > every.vector_macs, "{tokens}: the walk through a window more log");
        assert_eq!((trailing.weight_bytes, trailing.array_macs), (every.weight_bytes, every.array_macs), "{tokens}: the weights and the array");
    }
}

#[test]
fn one_word_activations_halve_the_array() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let one = price_serving(&h, &c, &plain(8)).unwrap();
    let two = price_serving(&h, &c, &Serving { activation_words: 2, ..plain(8) }).unwrap();
    assert!(close(two.array_macs, 2.0 * one.array_macs, 1e-12));
}

// ── The array ──────────────────────────────────────────────────────────────────────────────────────

/// Each 18×19 multiplier takes an 8-bit activation code `a` and two 4-bit weight codes twelve bits apart, and its
/// product holds `a·q₁` in bits 0..12 and `a·q₂` from bit 12, for every code: the lanes never touch.
#[test]
fn the_dsp_packing_keeps_its_two_products_apart() {
    for a in 0u64..256 {
        for q1 in 0u64..16 {
            for q2 in 0u64..16 {
                let packed = q1 + (q2 << 12);
                assert!(packed < 1 << 18 && a < 1 << 17, "the operands fit the multiplier's signed ports");
                let product = a * packed;
                assert!(product < 1 << 36, "the product fits the multiplier's 37-bit signed result");
                assert_eq!(product & 0xfff, a * q1, "a={a} q1={q1} q2={q2}: the low lane");
                assert_eq!(product >> 12, a * q2, "a={a} q1={q1} q2={q2}: the high lane");
            }
        }
    }
}

#[test]
fn a_weight_takes_one_pass_per_four_bit_digit_of_its_code() {
    use GgmlType::*;
    for (ty, passes) in [
        (Q2K, Some(1)),
        (Q3K, Some(1)),
        (Q4_0, Some(1)),
        (Q4_1, Some(1)),
        (Q4K, Some(1)),
        (Q5_0, Some(2)),
        (Q5_1, Some(2)),
        (Q5K, Some(2)),
        (Q6K, Some(2)),
        (Q8_0, Some(2)),
        (Iq4Nl, Some(2)),
        (Iq4Xs, Some(2)),
        (F32, None),
        (F16, None),
        (Bf16, None),
        (Q8K, None),
        (Iq2Xxs, None),
        (Mxfp4, None),
    ] {
        assert_eq!(array_passes(ty), passes, "{ty:?}");
    }
}

#[test]
fn every_ladder_checkpoint_rides_the_array_or_the_vector_units() {
    let c = card(&EnginePlan::standard());
    for (label, name, s) in ladder(0.924) {
        price_serving(&header(name), &c, &Serving { batch: 1, ..s }).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    }
}

#[test]
fn a_format_the_card_does_not_decode_is_refused_by_name() {
    let mut h = header("qwen3.8-27b-q4_k_m");
    let t = h.tensors.iter_mut().find(|t| t.name == "blk.0.ffn_up.weight").unwrap();
    t.type_id = GgmlType::Iq2Xxs.id();
    match price_serving(&h, &card(&EnginePlan::standard()), &plain(1)) {
        Err(ServingRefusal::Undecoded { tensor, ty }) => assert_eq!((tensor.as_str(), ty), ("blk.0.ffn_up.weight", GgmlType::Iq2Xxs)),
        other => panic!("an IQ2_XXS matrix must be refused, got {other:?}"),
    }
}

#[test]
fn a_mixture_of_experts_is_refused_not_mispriced() {
    match price_serving(&header("qwen3-30b-a3b-q8_0"), &card(&EnginePlan::standard()), &plain(1)) {
        Err(ServingRefusal::Routed) => {}
        other => panic!("a mixture of experts must be refused, got {other:?}"),
    }
}

#[test]
fn the_most_sequences_is_the_least_batch_refused_minus_one() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let n = max_batch(&h, &c, &plain(1)).unwrap();
    assert!(n > 1, "{n}");
    assert!(price_serving(&h, &c, &plain(n)).is_ok());
    assert!(price_serving(&h, &c, &plain(n + 1)).is_err());
}

// ── Contexts ───────────────────────────────────────────────────────────────────────────────────────

/// Every per-position cost is linear in position, so a paged mix of contexts prices exactly as its mean.
#[test]
fn a_paged_mix_of_contexts_prices_as_its_mean_position() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let mix = mixed();
    let longest = mix.longest();
    let at = |contexts: ContextMix| Serving { contexts, kv_page: Some(256), ..plain(30) };
    let a = price_serving(&h, &c, &at(mix.clone())).unwrap();
    let b = price_serving(&h, &c, &at(ContextMix::uniform(mix.mean_position(), longest))).unwrap();
    assert_eq!((a.sequence_bytes, a.vector_macs, a.array_macs, a.held_bytes), (b.sequence_bytes, b.vector_macs, b.array_macs, b.held_bytes));
    assert!(close(mix.mean_position(), 0.70 * 512.0 + 0.25 * 4096.0 + 0.05 * 65536.0, 1e-15));
}

#[test]
fn a_context_past_the_checkpoints_own_is_refused() {
    let h = header("qwen3.8-27b-q4_k_m");
    let model = h.meta_u64("context_length").unwrap();
    assert_eq!(model, 262_144);
    let c = card(&EnginePlan::standard());
    assert!(price_serving(&h, &c, &Serving { contexts: ContextMix::uniform(1.0, model), ..plain(1) }).is_ok());
    match price_serving(&h, &c, &Serving { contexts: ContextMix::uniform(1.0, model + 1), ..plain(1) }) {
        Err(ServingRefusal::ContextBeyondModel { limit, model: m }) => assert_eq!((limit, m), (model + 1, model)),
        other => panic!("a limit past the model must be refused, got {other:?}"),
    }
    for bad in [
        ContextMix { classes: vec![ContextClass { share: 0.5, position: 10.0, limit: 100 }] },
        ContextMix { classes: vec![ContextClass { share: 1.0, position: 101.0, limit: 100 }] },
        ContextMix { classes: vec![] },
    ] {
        match price_serving(&h, &c, &Serving { contexts: bad.clone(), ..plain(1) }) {
            Err(ServingRefusal::BadContexts(_)) => {}
            other => panic!("{bad:?} must be refused, got {other:?}"),
        }
    }
}

#[test]
fn attention_on_the_array_moves_its_products_and_leaves_the_softmax() {
    let h = header("qwen3.8-27b-q4_k_m");
    let vector = PassShape::of(&h, &plain(1)).unwrap();
    let array = PassShape::of(&h, &Serving { attention: AttentionMath::Array, ..plain(1) }).unwrap();
    // 16 attention layers, 24 query heads, a 256-element key and value each, at the mean position, one row.
    let products = 16.0 * 24.0 * 512.0 * 1024.0;
    assert!(close(vector.vector_macs - array.vector_macs, products, 1e-12), "{}", vector.vector_macs - array.vector_macs);
    assert!(close(array.array_macs - vector.array_macs, 2.0 * products, 1e-12), "two passes of an 8-bit cache code");
    let softmax = SOFTMAX_MACS_PER_SCORE * 16.0 * 24.0 * 1024.0;
    assert!(array.vector_macs > softmax, "the softmax stays on the vector units");
    match PassShape::of(&h, &Serving { attention: AttentionMath::Array, kv_bytes: 2.0, ..plain(1) }) {
        Err(ServingRefusal::WideCacheOnArray { .. }) => {}
        other => panic!("an f16 cache on the integer array must be refused, got {other:?}"),
    }
}

/// A longer context never serves faster, and serves strictly slower wherever it binds on a term the cache's length
/// moves — the memory, the host link or the vector units. Two contexts the array binds both serve at its ceiling.
#[test]
fn throughput_never_rises_as_the_context_grows() {
    let r = roofline(&decode_dir()).unwrap();
    for name in ["vector units", "array"] {
        let sweep: Vec<_> = CONTEXT_SWEEP
            .iter()
            .map(|limit| {
                let label = format!("context {} of {limit}, attention on the {name}", limit / 2);
                r.rows.iter().find(|row| row.label == label).unwrap_or_else(|| panic!("no row `{label}`"))
            })
            .collect();
        for pair in sweep.windows(2) {
            let (short, long) = (pair[0], pair[1]);
            assert!(long.tokens_per_s <= short.tokens_per_s, "`{}` is faster than `{}`", long.label, short.label);
            if matches!(long.cost.binds, ServingBind::Memory | ServingBind::Host | ServingBind::Vector) {
                assert!(
                    long.tokens_per_s < short.tokens_per_s,
                    "`{}` binds on {:?} and is not slower than `{}`",
                    long.label,
                    long.cost.binds,
                    short.label
                );
            }
        }
        assert!(sweep.last().unwrap().tokens_per_s < sweep[0].tokens_per_s, "the {name}'s sweep never falls");
        let longest = sweep.last().unwrap();
        assert!(longest.cost.batch >= 1 && longest.serving.contexts.longest() == 262_144, "{}", longest.label);
    }
}

#[test]
fn attention_on_the_array_wins_once_the_cache_is_long() {
    let r = roofline(&decode_dir()).unwrap();
    for limit in CONTEXT_SWEEP.iter().filter(|l| **l >= 16_384) {
        let find = |name: &str| {
            let label = format!("context {} of {limit}, attention on the {name}", limit / 2);
            r.rows.iter().find(|row| row.label == label).unwrap().tokens_per_s
        };
        assert!(find("array") > find("vector units"), "at {limit}");
    }
}

// ── The accounting, term by term ───────────────────────────────────────────────────────────────────

/// Every per-sequence byte of a drafted pass, from the checkpoint's own dimensions, under both ways of carrying the
/// recurrent state.
#[test]
fn a_drafted_pass_moves_and_holds_exactly_its_terms() {
    let h = header("qwen3.8-27b-q4_k_m");
    let meta = |k: &str| h.meta_u64(k).unwrap() as f64;
    let (heads, key, inner, groups, taps) =
        (meta("ssm.time_step_rank"), meta("ssm.state_size"), meta("ssm.inner_size"), meta("ssm.group_count"), meta("ssm.conv_kernel"));
    let (sb, kv) = (2.0, 1.0);
    let dense = heads * key * (inner / heads) * sb;
    let channels = 2.0 * groups * key + inner;
    let (ring, ring_row) = ((taps - 1.0) * channels * sb, channels * sb);
    let log = (groups * key + inner + 2.0 * heads) * sb;
    let per_position = 4.0 * 512.0 * kv;
    let (gdn, attn) = (48.0, 16.0);
    let (p, limit) = (700.0, 4096u64);
    for k in [1u32, 2, 3] {
        let acceptance: Vec<_> = (0..k).map(|_| measured_acceptance(0.8)).collect();
        let e = emitted_per_pass(&vec![0.8; k as usize]);
        let n = 1.0 + k as f64;
        let kf = k as f64;
        for flush in [StateFlush::EveryPass, StateFlush::Every { tokens: 16 }, StateFlush::Trailing { tokens: 8 }] {
            let s = Serving {
                contexts: ContextMix::uniform(p, limit),
                draft: Some(Draft { tokens: k, acceptance: acceptance.clone(), head_rows: None }),
                flush,
                ..plain(1)
            };
            let shape = PassShape::of(&h, &s).unwrap();
            let (state_read, state_write, state_held) = match flush {
                StateFlush::EveryPass => (0.0, n * dense, n * dense),
                StateFlush::Every { tokens } => {
                    let t = tokens as f64;
                    let window = ((t - e) / 2.0).max(0.0);
                    (window * log, n * log + e / t * dense, dense + (t + n) * log)
                }
                StateFlush::Trailing { tokens } => {
                    let t = tokens as f64;
                    let window = t + ((t - e) / 2.0).max(0.0);
                    (window * log, n * log + e / t * dense, dense + (2.0 * t + n) * log)
                }
            };
            let read = gdn * (dense + ring + state_read) + attn * per_position * p + kf * per_position * p;
            let write = gdn * (ring_row * n + state_write) + attn * per_position * n + per_position * kf;
            let held = gdn * (ring + ring_row * n + state_held) + (attn + 1.0) * per_position * limit as f64;
            assert!(close(shape.read_bytes, read, 1e-12), "k={k} {flush:?}: read {} vs {read}", shape.read_bytes);
            assert!(close(shape.write_bytes, write, 1e-12), "k={k} {flush:?}: write {} vs {write}", shape.write_bytes);
            assert!(close(shape.held_per_sequence, held, 1e-12), "k={k} {flush:?}: held {} vs {held}", shape.held_per_sequence);
            let row = 5120.0 * 144.0 / 256.0;
            assert!(close(shape.embed_bytes, row * (n + kf), 1e-12), "k={k}: an embedding row per verified row and per drafting step");
        }
    }
}

#[test]
fn the_token_rows_on_chip_are_their_residuals_and_widest_activations() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    for (words, k) in [(1u32, 0u32), (2, 0), (1, 2)] {
        let s = Serving {
            activation_words: words,
            draft: (k > 0).then(|| Draft { tokens: k, acceptance: vec![measured_acceptance(0.8); k as usize], head_rows: None }),
            flush: StateFlush::Every { tokens: 16 },
            ..plain(1)
        };
        let shape = PassShape::of(&h, &s).unwrap();
        let rows = 1.0 + k as f64;
        assert_eq!(shape.on_chip_per_sequence, rows * (4.0 * 5120.0 + words as f64 * (17408.0 + 5120.0)), "words {words}, k {k}");
        let mut small = c.clone();
        small.on_chip.capacity.value = 10.5 * shape.on_chip_per_sequence;
        assert!(shape.cost(&small, 10, 0).is_ok());
        match shape.cost(&small, 11, 0) {
            Err(ServingRefusal::OnChipOverflow { needed, capacity }) => assert!(needed > capacity),
            other => panic!("eleven sequences past a ten-and-a-half-sequence chip must be refused, got {other:?}"),
        }
    }
}

#[test]
fn the_softmax_stays_on_the_vector_units_at_its_cost_per_score() {
    let h = header("qwen3.8-27b-q4_k_m");
    let at = |p: f64| PassShape::of(&h, &Serving { attention: AttentionMath::Array, contexts: ContextMix::uniform(p, 4096), ..plain(1) }).unwrap();
    // Sixteen attention layers of twenty-four query heads, one score per head per position, at 7.5 slots a score.
    let per_position = 7.5 * 16.0 * 24.0;
    assert_eq!(SOFTMAX_MACS_PER_SCORE, 7.5, "fifteen f32 operations a score, two to a slot");
    assert!(close(at(1000.0).vector_macs - at(0.0).vector_macs, 1000.0 * per_position, 1e-12));
}

// ── The block-scale lanes ──────────────────────────────────────────────────────────────────────────

/// A lane spends a cycle on a block whose sub-block multipliers fit its sixteen products, two on Q2_K's thirty-two;
/// activations scaled per 32 elements give a 256-element block eight aligned terms, a cycle each.
#[test]
fn a_lane_spends_a_cycle_a_block_and_one_per_32_elements_of_finer_activations() {
    use GgmlType::*;
    assert_eq!(LANE_PRODUCTS, 16.0);
    for (ty, per_block, sub32) in [
        (Q4_0, 1.0, 1.0),
        (Q4_1, 1.0, 1.0),
        (Q5_0, 1.0, 1.0),
        (Q5_1, 1.0, 1.0),
        (Q8_0, 1.0, 1.0),
        (Iq4Nl, 1.0, 1.0),
        (Iq4Xs, 1.0, 8.0),
        (Q3K, 1.0, 8.0),
        (Q4K, 1.0, 8.0),
        (Q5K, 1.0, 8.0),
        (Q6K, 1.0, 8.0),
        (Q2K, 2.0, 8.0),
    ] {
        assert_eq!(lane_cycles(ty, ActScaling::PerBlock), Some(per_block), "{ty:?} per block");
        assert_eq!(lane_cycles(ty, ActScaling::Sub32), Some(sub32), "{ty:?} per 32");
        assert_eq!(lane_cycles(ty, ActScaling::PerBlock).is_some(), array_passes(ty).is_some(), "{ty:?}: lanes for every type the array reads");
    }
    for ty in [F32, F16, Bf16, Mxfp4] {
        assert_eq!(lane_cycles(ty, ActScaling::PerBlock), None, "{ty:?}");
    }
}

/// The standard plan holds the fewest lanes that keep up with its array at [`LANE_MIX`] array cycles a lane cycle,
/// and what the lanes take comes off the array's DSP blocks; the plan stays inside the device. Its target keeps
/// the lanes no busier than that: Qwen3.8-27B Q4_K_M, drafting or not, spends at least [`LANE_MIX`] array cycles per
/// lane cycle, so its lanes never bind before its array.
#[test]
fn the_plan_holds_the_fewest_lanes_its_target_fills() {
    let plan = EnginePlan::standard();
    let macs = |p: &EnginePlan| p.array_dsp_blocks as f64 * ARRAY_MACS_PER_DSP as f64 + p.array_soft_macs.value;
    assert!(plan.scale_lanes as f64 * LANE_MIX >= macs(&plan), "{} lanes for {} multiply-adds a cycle", plan.scale_lanes, macs(&plan));
    let fewer = EnginePlan { scale_lanes: plan.scale_lanes - 1, array_dsp_blocks: plan.array_dsp_blocks + SCALE_LANE_DSPS, ..plan };
    assert!((fewer.scale_lanes as f64) * LANE_MIX < macs(&fewer), "one lane fewer falls behind its array");
    assert_eq!(plan.dsp_blocks(), plan.array_dsp_blocks + SCALE_LANE_DSPS * plan.scale_lanes + plan.vector_dsp_blocks);
    assert!(plan.dsp_blocks() <= DSP_BLOCKS && plan.alms() <= ALMS as f64);
    let c = card(&plan);
    assert_eq!(c.scale.value, plan.scale_lanes as f64 * plan.clock_hz.value);
    let h = header("qwen3.8-27b-q4_k_m");
    for draft in [None, Some(Draft { tokens: 1, acceptance: vec![measured_acceptance(0.924)], head_rows: Some(32_768) })] {
        let shape = PassShape::of(&h, &Serving { draft: draft.clone(), flush: StateFlush::Every { tokens: 16 }, ..plain(1) }).unwrap();
        let mix = shape.array_macs / shape.scale_cycles;
        assert!(mix >= LANE_MIX, "drafting {}: {mix:.1} array cycles a lane cycle", draft.is_some());
    }
}

/// The lanes are their own term: a pass spends, for each sequence, a block's lane cycles per weight block per word
/// per row it multiplies, beside the array's multiply-adds.
#[test]
fn a_pass_spends_its_lane_cycles_beside_its_multiply_adds() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    for (words, scaling) in [(1u32, ActScaling::PerBlock), (2, ActScaling::PerBlock), (1, ActScaling::Sub32)] {
        let s = Serving { activation_words: words, scaling, ..plain(8) };
        let shape = PassShape::of(&h, &s).unwrap();
        let cost = shape.cost(&c, 8, 0).unwrap();
        assert!(close(cost.scale_cycles, 8.0 * shape.scale_cycles, 1e-12));
        assert!(close(cost.scale_s, cost.scale_cycles / c.scale.value, 1e-12));
        // Every block-format matrix's blocks, once a word, at its lane cycles: the expected cycles from the header.
        let mut want = 0.0;
        for t in &h.tensors {
            let ty = h.tensor_type(t).unwrap();
            let is_matrix = t.dims.len() >= 2 && !t.name.contains("norm");
            let draft = t.name.starts_with("blk.64.");
            if is_matrix && array_passes(ty).is_some() && !draft && t.name != "token_embd.weight" {
                let elems: f64 = t.dims.iter().product::<u64>() as f64;
                want += elems / ty.block_elems() as f64 * lane_cycles(ty, scaling).unwrap() * words as f64;
            }
        }
        assert!(close(shape.scale_cycles, want, 1e-9), "{words} words {scaling:?}: {} vs {want}", shape.scale_cycles);
    }
}

/// Activations scaled per 32 elements bind a K-quant serving to its lanes at a batch where per-block scaling leaves
/// the array or the memory binding.
#[test]
fn per_32_activations_bind_a_k_quant_serving_to_its_lanes() {
    let h = header("qwen3.8-27b-q4_k_m");
    let c = card(&EnginePlan::standard());
    let fine = price_serving(&h, &c, &Serving { scaling: ActScaling::Sub32, ..plain(64) }).unwrap();
    let coarse = price_serving(&h, &c, &Serving { scaling: ActScaling::PerBlock, ..plain(64) }).unwrap();
    assert_eq!(fine.binds, ServingBind::Scale);
    assert_ne!(coarse.binds, ServingBind::Scale);
    // Eight cycles a 256-element block instead of one; the checkpoint's Q8_0 and Q4_0 matrices cost a cycle per
    // 32 elements either way.
    let ratio = fine.scale_s / coarse.scale_s;
    assert!(ratio > 4.0 && ratio < 8.0, "per-32 activations cost {ratio:.2} times the lane cycles");
}

/// Attention on the array rounds every score and every value element's sum over each block of positions on a lane.
#[test]
fn attention_on_the_array_rounds_its_scores_and_sums_on_the_lanes() {
    let h = header("qwen3.8-27b-q4_k_m");
    let at = |attention| PassShape::of(&h, &Serving { attention, contexts: ContextMix::uniform(4096.0, 8192), ..plain(1) }).unwrap();
    let (vector, array) = (at(AttentionMath::Vector), at(AttentionMath::Array));
    // 16 attention layers, 24 query heads, 4,096 positions; a score each, and 256 value elements per 32 positions.
    let want = 16.0 * 24.0 * 4096.0 * (1.0 + 256.0 / VALUE_BLOCK);
    assert!(close(array.scale_cycles - vector.scale_cycles, want, 1e-12), "{}", array.scale_cycles - vector.scale_cycles);
}

/// A rotated cache costs its transforms on the vector units: per attention layer and row, every key and value head
/// of the cache and of the queries, `n·log2 n` additions each.
#[test]
fn a_rotated_cache_costs_its_transforms_on_the_vector_units() {
    let h = header("qwen3.8-27b-q4_k_m");
    let at = |kv_rotated| PassShape::of(&h, &Serving { kv_rotated, ..plain(1) }).unwrap();
    let (plain_shape, rotated) = (at(false), at(true));
    let transform = 256.0 * 8.0;
    let want = 16.0 * (4.0 + 24.0) * 2.0 * transform;
    assert!(close(rotated.vector_macs - plain_shape.vector_macs, want, 1e-12), "{}", rotated.vector_macs - plain_shape.vector_macs);
    assert_eq!(rotated.array_macs, plain_shape.array_macs);
    assert_eq!(rotated.read_bytes, plain_shape.read_bytes, "the rotation changes no byte a pass moves");
}

/// Reading all 72 bits of every beat is nine bytes a transfer and nine gibibytes a channel, from the shell's own
/// controller configuration; the data-sheet plan stays at eight.
#[test]
fn the_full_beat_carries_nine_bytes_a_transfer() {
    let full = card(&EnginePlan::full_beat());
    assert_eq!(EnginePlan::full_beat().beat, Beat::Full72);
    assert_eq!(EnginePlan::standard().beat, Beat::Data64);
    assert_eq!(full.memory.bandwidth.value, 4.0 * 2400e6 * 9.0);
    assert_eq!(full.memory.capacity.value, 4.0 * 9.0 * (1u64 << 30) as f64);
    assert!(full.memory.bandwidth.cite.contains("CTRL_DDR4_ECC_EN false") && full.memory.bandwidth.cite.contains("AVMM_DATA_WIDTH 576"));
    assert_cited("d5005 full-beat card", &full.facts(), full.sources);
    let r = roofline(&decode_dir()).unwrap();
    assert!(!r.full_beat.rows.is_empty());
    for row in &r.full_beat.rows {
        let base = r.rows.iter().find(|b| row.label.starts_with(&b.label)).expect("every full-beat row repeats a standard row");
        assert!(row.tokens_per_s >= base.tokens_per_s * (1.0 - 1e-12), "{}: {} vs {}", row.label, row.tokens_per_s, base.tokens_per_s);
        let again = price_serving(&header(row.header), &full, &row.serving).unwrap();
        assert!(close(again.tokens_per_s(), row.tokens_per_s, 1e-12), "{}", row.label);
    }
}

/// The card's device constants are the part entry's facts, and the on-chip bandwidth is every M20K block's one widest
/// read port.
#[test]
fn the_device_constants_are_the_part_entrys() {
    let part = PartResources::sx2800();
    let m20k = part.memory(MemoryKind::M20k).unwrap();
    assert_eq!(
        (DSP_BLOCKS as u64, ALMS as u64, M20K_BLOCKS as u64, M20K_BITS as u64),
        (part.dsp.value, part.logic.value, m20k.count.value, m20k.bits.value)
    );
    let plan = EnginePlan::standard();
    let c = card(&plan);
    assert_eq!(c.on_chip.bandwidth.value, M20K_BLOCKS as f64 * m20k.read_bits_per_cycle.value as f64 / 8.0 * plan.clock_hz.value);
    assert_eq!(m20k.read_bits_per_cycle.value, 40);
}
