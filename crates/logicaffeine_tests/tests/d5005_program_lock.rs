//! The decode engine's program: a checkpoint's step as the commands the engine's units run.
//!
//! `spatial::decode_engine::program::lower` reads a qwen35 step at a knob point off the model's one program, and
//! `EngineModel` runs exactly it.
//! These tests hold the program's shape on the tiny model and, where they are downloaded, on the real checkpoints:
//! the weight stream it reads — every tensor of the model's blocks, each once a step, in layer order, the
//! multi-token-prediction blocks left to the draft; the dataflow between its commands — every slot a command reads
//! written before it in the step; the state each block carries owned by its own commands; the rotated cache's
//! transforms present exactly at the knob point that rotates; the model the program of its own knob point; and the
//! sequencer's words decoding to the program.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::{BTreeSet, HashSet};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::model::{CacheFormat, EngineModel, EngineNumerics};
use logicaffeine_compile::spatial::decode_engine::program::{lower, Command, Program, Slot};
use tiny_models::Tiny;

/// The checkpoints the program is held on, each with its architecture and its tensors' names: the tiny qwen35 model
/// and the 27B's header ledgers always, the real checkpoints where they are downloaded (only their headers read).
fn checkpoints(dir: &std::path::Path) -> Vec<(String, ModelSpec, Vec<String>)> {
    let names = |g: &Gguf| g.tensors().iter().map(|t| t.name.clone()).collect::<Vec<_>>();
    let tiny = dir.join("qwen35.gguf");
    let spec = Tiny::Qwen35.wide(64);
    tiny_models::write_checkpoint(&spec, &tiny);
    let mut out = vec![("the tiny qwen35".to_string(), spec, names(&Gguf::open(&tiny).unwrap()))];
    for ledger in ["qwen3.8-27b-q4_k_m", "qwen3.8-27b-q8_0"] {
        let h = GgufHeader::read_ledger(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../benchmarks/silicon/decode/headers/{ledger}.json"))).unwrap();
        out.push((format!("the {ledger} header"), h.spec().unwrap(), h.tensors.iter().map(|t| t.name.clone()).collect()));
    }
    for name in ["qwen3.5-0.8b", "qwen3.8-27b/Qwen3.8-27B-Q4_K_M.gguf"] {
        if common::tool_available(&format!("checkpoint:{name}")) {
            let g = Gguf::open(&common::checkpoint(name).unwrap()).unwrap();
            out.push((name.to_string(), ModelSpec::from_gguf(&g).unwrap(), names(&g)));
        }
    }
    out
}

fn tensors_read(p: &Program) -> Vec<String> {
    p.tensors().into_iter().map(String::from).collect()
}

/// The step reads every tensor of the model's blocks, the embedding, the final norm and the head, and nothing else:
/// each once, the blocks' in layer order, the blocks of multi-token prediction (the draft's) left out.
#[test]
fn the_step_reads_every_tensor_of_the_model_once_in_layer_order() {
    let dir = tempfile::tempdir().unwrap();
    for (name, spec, tensors) in checkpoints(dir.path()) {
        let layers = spec.layers.len();
        let in_a_block = |t: &str| -> Option<usize> { t.strip_prefix("blk.")?.split('.').next()?.parse().ok() };
        let want: BTreeSet<String> = tensors.into_iter().filter(|t| in_a_block(t).is_none_or(|l| l < layers)).collect();
        for n in EngineNumerics::all() {
            let read = tensors_read(&lower(&spec, n, spec.ctx_len).unwrap().program);
            let set: BTreeSet<String> = read.iter().cloned().collect();
            assert_eq!(set.len(), read.len(), "{name}: a tensor named twice");
            let missing: Vec<&String> = want.difference(&set).collect();
            let extra: Vec<&String> = set.difference(&want).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{name} at {n:?}: the step leaves {missing:?} unread and reads {extra:?}, which the checkpoint does not hold");
            let blocks: Vec<usize> = read.iter().filter_map(|t| in_a_block(t)).collect();
            assert!(blocks.windows(2).all(|w| w[0] <= w[1]), "{name}: the blocks' tensors out of layer order");
            assert_eq!(read.first().map(String::as_str), Some("token_embd.weight"), "{name}: the step starts at the embedding");
        }
    }
}

/// Every matrix streams once a step: no two matrix-vector commands name one tensor, and the count is each block's
/// projections plus the head.
#[test]
fn every_matrix_streams_once_a_step() {
    let dir = tempfile::tempdir().unwrap();
    for (name, spec, _) in checkpoints(dir.path()) {
        let p = lower(&spec, EngineNumerics::fastest(), spec.ctx_len).unwrap().program;
        let matrices: Vec<&str> = p.commands.iter().filter_map(|c| if let Command::Matvec { tensor, .. } = c { Some(tensor.as_str()) } else { None }).collect();
        let unique: HashSet<&str> = matrices.iter().copied().collect();
        assert_eq!(unique.len(), matrices.len(), "{name}: a matrix streamed twice in one step");
        let per_block: usize = spec
            .layers
            .iter()
            .map(|l| {
                let mixer = match l.mixer {
                    Mixer::GatedDelta(_) => 5,
                    Mixer::Attention(_) => 4,
                    Mixer::Mla(_) => unreachable!("not a qwen35 block"),
                };
                mixer + 3
            })
            .sum();
        assert_eq!(matrices.len(), per_block + 1, "{name}: each block's projections and the FFN's three, and the head");
    }
}

/// Every slot a command reads is written earlier in the step, and the step ends with the logits written.
#[test]
fn every_slot_a_command_reads_is_written_before_it() {
    let dir = tempfile::tempdir().unwrap();
    for (name, spec, _) in checkpoints(dir.path()) {
        for n in EngineNumerics::all() {
            let p = lower(&spec, n, spec.ctx_len).unwrap().program;
            let mut written: HashSet<Slot> = HashSet::new();
            for (i, c) in p.commands.iter().enumerate() {
                let (reads, writes) = Program::dataflow(c);
                for s in reads {
                    assert!(written.contains(&s), "{name} at {n:?}: command {i} ({c:?}) reads {s:?} before the step writes it");
                }
                written.extend(writes);
            }
            assert!(written.contains(&Slot::Logits), "{name}: the step never writes its logits");
            assert!(matches!(p.commands.last(), Some(Command::Matvec { to: Slot::Logits, .. })), "{name}: the head is the step's last command");
        }
    }
}

/// The state a block carries is its own commands': a Gated-DeltaNet block's ring by its convolution and its memory by
/// its delta, an attention block's cache by its append and its attend, each once a step, and the positions move once,
/// after every block has read them.
#[test]
fn each_blocks_state_is_carried_by_its_own_commands() {
    let dir = tempfile::tempdir().unwrap();
    for (name, spec, _) in checkpoints(dir.path()) {
        let p = lower(&spec, EngineNumerics::precise(), spec.ctx_len).unwrap().program;
        for (l, layer) in spec.layers.iter().enumerate() {
            let count = |f: &dyn Fn(&Command) -> bool| p.commands.iter().filter(|c| f(c)).count();
            let (convs, deltas) = (count(&|c| matches!(c, Command::Conv { layer, .. } if *layer == l)), count(&|c| matches!(c, Command::Delta { layer, .. } if *layer == l)));
            let (appends, attends) = (count(&|c| matches!(c, Command::CacheAppend { layer, .. } if *layer == l)), count(&|c| matches!(c, Command::Attend { layer, .. } if *layer == l)));
            match layer.mixer {
                Mixer::GatedDelta(_) => assert_eq!((convs, deltas, appends, attends), (1, 1, 0, 0), "{name}: block {l}'s ring and memory"),
                Mixer::Attention(_) => assert_eq!((convs, deltas, appends, attends), (0, 0, 1, 1), "{name}: block {l}'s cache"),
                Mixer::Mla(_) => unreachable!("not a qwen35 block"),
            }
        }
        let advances: Vec<usize> = p.commands.iter().enumerate().filter(|(_, c)| matches!(c, Command::Advance)).map(|(i, _)| i).collect();
        assert_eq!(advances.len(), 1, "{name}: the positions move once a step");
        let last_reader = p.commands.iter().rposition(|c| matches!(c, Command::Rotate { .. } | Command::Attend { .. })).unwrap();
        assert!(advances[0] > last_reader, "{name}: the positions move before a block has read them");
    }
}

/// The Walsh–Hadamard transforms are in the step exactly when the cache is rotated: the keys and the values before
/// they are cached, the queries before they attend.
#[test]
fn a_rotated_cache_turns_keys_values_and_queries() {
    let spec = Tiny::Qwen35.wide(64);
    let attention_blocks = spec.layers.iter().filter(|l| matches!(l.mixer, Mixer::Attention(_))).count();
    for n in EngineNumerics::all() {
        let p = lower(&spec, n, spec.ctx_len).unwrap().program;
        let turns: Vec<Slot> = p.commands.iter().filter_map(|c| if let Command::Hadamard { slot, .. } = c { Some(*slot) } else { None }).collect();
        if n.cache == CacheFormat::Int8Hadamard {
            assert_eq!(turns.len(), 3 * attention_blocks, "{n:?}: three transforms an attention block");
            for (i, c) in p.commands.iter().enumerate() {
                if let Command::CacheAppend { layer, .. } = c {
                    let before: Vec<Slot> = p.commands[..i].iter().rev().take_while(|c| !matches!(c, Command::Norm { .. })).filter_map(|c| if let Command::Hadamard { slot, .. } = c { Some(*slot) } else { None }).collect();
                    assert!(before.contains(&Slot::Key) && before.contains(&Slot::Value), "{n:?}: block {layer} caches unrotated rows");
                    assert!(matches!(p.commands[i + 1], Command::Hadamard { slot: Slot::Query, .. }), "{n:?}: block {layer}'s queries attend unrotated");
                }
            }
        } else {
            assert!(turns.is_empty(), "{n:?}: a Walsh–Hadamard transform with an unrotated cache");
        }
    }
}

/// The model runs the program of its own knob point.
#[test]
fn the_model_runs_the_program_of_its_knob_point() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("qwen35.gguf");
    let spec = Tiny::Qwen35.wide(64);
    tiny_models::write_checkpoint(&spec, &path);
    for n in EngineNumerics::all() {
        let e = EngineModel::new(spec.clone(), Gguf::open(&path).unwrap(), n).unwrap();
        assert_eq!(e.program(), &lower(&spec, n, spec.ctx_len).unwrap().program, "{n:?}");
    }
}

/// The sequencer's words are the program: every knob point's step on every checkpoint encodes into words whose
/// decoding, against the program's tensors and rotary tables, is the program command for command.
#[test]
fn the_sequencers_words_decode_to_the_program() {
    let dir = tempfile::tempdir().unwrap();
    for (name, spec, _) in checkpoints(dir.path()) {
        for n in EngineNumerics::all() {
            let p = lower(&spec, n, spec.ctx_len).unwrap().program;
            let words = p.encode();
            assert_eq!(words.len(), p.commands.len(), "{name}: a word a command");
            assert_eq!(Program::decode(&words, &p.tensors(), &p.ropes()), p.commands, "{name} at {n:?}: the words decode to another program");
        }
    }
}

/// Every field of a word round-trips at its extremes: a program reading more tensors than 2^19, so the tensor fields'
/// top bits carry, with counts of 65,535, offsets of 2^20 − 1, layer 255, every slot, and a constant of arbitrary bits.
#[test]
fn every_field_of_a_word_round_trips_at_its_extremes() {
    use Slot::*;
    let slots = [Residual, Normed, Out, Qkv, Mixed, Z, Alpha, Beta, LogDecay, Decay, Gate, Query, Key, Value, Projected, OutGate, Read, FfnGate, FfnUp, Hidden, Logits];
    let mut commands: Vec<Command> = (0..(1usize << 19) + 3).map(|i| Command::Matvec { tensor: format!("t{i}"), from: slots[i % slots.len()], to: slots[(i + 1) % slots.len()] }).collect();
    let rope = Tiny::Qwen35.wide(64).layers.iter().find_map(|l| if let Mixer::Attention(a) = &l.mixer { a.rope.clone() } else { None }).unwrap();
    commands.extend([
        Command::Gates { alpha: Hidden, beta: Logits, dt: "t524290".into(), a: "t524289".into(), log_decay: FfnUp, decay: FfnGate, gate: OutGate },
        Command::Delta {
            layer: 255,
            query: Query,
            key: Key,
            value: Value,
            value_at: (1 << 20) - 1,
            log_decay: LogDecay,
            decay: Decay,
            gate: Gate,
            k_heads: 65_535,
            v_heads: 65_534,
            width: 65_533,
            to: Logits,
        },
        Command::L2Heads { from: Projected, at: (1 << 20) - 1, heads: 65_535, width: 32_768, to: OutGate },
        Command::Attend { layer: 255, query: Hidden, heads: 65_535, kv_heads: 32_768, width: 65_535, value_width: 1, scale: f32::from_bits(0xdead_beef), to: Logits },
        Command::CacheAppend { layer: 254, key: Logits, value: Hidden, kv_heads: 65_535, width: 65_535, value_width: 65_535 },
        Command::Conv { layer: 255, kernel: "t0".into(), taps: 65_535, from: Logits, to: Residual },
        Command::Rotate { slot: Logits, width: 65_535, rope },
        Command::Advance,
    ]);
    let p = Program { numerics: EngineNumerics::fastest(), commands };
    assert_eq!(Program::decode(&p.encode(), &p.tensors(), &p.ropes()), p.commands);
}
