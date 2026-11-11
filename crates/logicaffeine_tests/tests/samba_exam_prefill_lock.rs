//! samba_exam_prefill_lock — the rows of the Samba book's exam E4 (section 10.4 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! prefill and the first token) that hold today.
//!
//! A prompt is prefilled by the authored prefill program a chunk of its rows at a time: each chunk's program is given the
//! chunk's tokens (`toks`), its first position (`pos_s0`) and the state the chunk before it left — every tensor the
//! decode's contract carries (`spatial::handoff`) — and leaves that state for the next. The programs run through the
//! spatial evaluator as a host runs them (`common/samba_handoff.rs`), on the SN50 and on the GPU target.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_prefill_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use logicaffeine_compile::model::program::Cut;
use logicaffeine_compile::spatial::handoff::contract;
use logicaffeine_compile::spatial::target::SpatialTarget;
use samba_handoff::{authored, run, Model, CTX};
use std::collections::BTreeMap;
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

/// `prompt` prefilled on `target` in chunks of `chunks` rows, one after another, each chunk's program given the state the
/// one before it left: the state the last leaves, by the contract's names.
fn prefilled_in(m: &Model, target: &SpatialTarget, prompt: &[u32], chunks: &[usize]) -> BTreeMap<String, Vec<f64>> {
    assert_eq!(chunks.iter().sum::<usize>(), prompt.len(), "the chunks cover the prompt");
    let c = contract(&m.spec, CTX).expect("the contract");
    let mut state = BTreeMap::new();
    let mut first = 0usize;
    for &rows in chunks {
        let p = authored(m, Cut::Prefill { rows, ctx: CTX }, target);
        let mut given = state.clone();
        // The program counts tokens and positions from 1.
        given.insert("toks".to_string(), prompt[first..first + rows].iter().map(|t| f64::from(t + 1)).collect());
        given.insert("pos_s0".to_string(), vec![(first + 1) as f64]);
        let out = run(m, &p, &|n| Some(n.to_string()), &given);
        state = c.carried.iter().map(|t| (t.name.clone(), out[&t.name].clone())).collect();
        first += rows;
    }
    state
}

/// E4.4 — chunks whose row counts no tile height divides: a prompt of 7 tokens prefilled as 3 then 4, 1 then 6, 5 then 2,
/// and 2, 2 then 3 leaves every carried tensor the one chunk of 7 leaves, value for value, on the SN50 (32-row tiles) and
/// on the GPU target (16-row tiles). The one chunk's state is the reference's as E2's rows hold a prompt's.
#[test]
fn ragged_chunks_leave_the_state_one_chunk_leaves() {
    let splits: [&[usize]; 4] = [&[3, 4], &[1, 6], &[5, 2], &[2, 2, 3]];
    for target in [SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
        assert!(
            (1..CTX).all(|rows| rows % target.tile.rows != 0),
            "{}: no chunk here is a whole number of tile heights",
            target.name
        );
        for t in SHARED {
            let m = Model::write(t.spec(CTX), &format!("{t:?}"));
            let prompt: Vec<u32> = (0..CTX as u32 - 1).map(|i| (i * 5 + 3) % m.spec.vocab as u32).collect();
            let whole = prefilled_in(&m, &target, &prompt, &[prompt.len()]);
            assert!(whole.values().all(|v| v.iter().any(|x| *x != 0.0)), "{t:?} on {}: every carried tensor holds the prompt", target.name);
            for chunks in splits {
                let state = prefilled_in(&m, &target, &prompt, chunks);
                for (name, want) in &whole {
                    assert_eq!(&state[name], want, "{t:?} on {}, chunks {chunks:?}: `{name}` is the one chunk's, value for value", target.name);
                }
            }
        }
    }
}
