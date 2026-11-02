//! A model's one program. [`author`] writes a model's computation as a LOGOS program, and every backend lowers that
//! program and no other: the CUDA lane compiles its decode step and prefill chunks, the D5005 engine reads its decode
//! step, a spatial chip lowers its embedding, its layers and its head one program each. A backend never grows a model
//! writer of its own, and takes every cut it lowers from this module.
//!
//! The program a backend lowers is a [`Cut`] of it — the decode step over a cache of `ctx` positions, a prefill chunk
//! of `rows` tokens, a batch of `rows` sequences, or one layer, the embedding or the head of any of them — written by
//! [`Cut::write`] with the placeholder inputs every backend replaces at run time (each token and position `1`, weights
//! named by tensor alone). A cut split over chips is written by [`written_split`]. Its [`Identity`] is the cut, the
//! chips it is split over, and the SHA-256 of the text `author` wrote for it; every artifact a backend compiles records
//! it, so an artifact names the exact program it was lowered from, and a program another writer produced cannot pass
//! for this one.

use std::fmt;

use super::author::{self, Authored, NotYet, RowState, TpAuthored};
use super::spec::ModelSpec;
use crate::crypto_accel::sha256;

/// Which cut of a model's program a backend lowers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cut {
    /// One decode step over a cache of `ctx` positions ([`author::decode_step`]).
    Decode { ctx: usize },
    /// A prefill chunk of `rows` tokens of one sequence over a cache of `ctx` positions ([`author::prefill`]).
    Prefill { rows: usize, ctx: usize },
    /// One decode step of `rows` sequences at once, each with its own state ([`author::decode_batch`]).
    Batch { rows: usize, ctx: usize },
    /// The single token's forward with no history ([`author::forward`]).
    Forward,
    /// Layer `layer` of the forward alone ([`author::layer`]).
    ForwardLayer { layer: usize },
    /// The forward's final norm and head alone ([`author::head`]).
    ForwardHead,
    /// A decode step's embedding lookup alone ([`author::decode_embed`]).
    DecodeEmbed,
    /// Layer `layer` of a decode step alone ([`author::decode_layer`]).
    DecodeLayer { layer: usize, ctx: usize },
    /// A decode step's final norm, head and arg-max alone ([`author::decode_head`]).
    DecodeHead,
    /// The embedding lookups of `rows` stacked rows alone ([`author::decode_batch_embed`]).
    BatchEmbed { rows: usize },
    /// Layer `layer` of a batch of `rows` sequences alone ([`author::stacked_layer`], each row its own sequence).
    BatchLayer { layer: usize, rows: usize, ctx: usize },
    /// A batch's head alone, every sequence's next token ([`author::decode_batch_head`]).
    BatchHead { rows: usize },
    /// Layer `layer` of a prefill chunk of `rows` tokens alone ([`author::stacked_layer`], one sequence's rows).
    PrefillLayer { layer: usize, rows: usize, ctx: usize },
    /// A prefill chunk's head alone, read off its last row ([`author::prefill_head`]).
    PrefillHead { rows: usize },
}

impl Cut {
    /// The program `author` writes for this cut of `spec`'s computation, its weight manifest naming files under `dir`
    /// (the text does not depend on `dir`).
    pub fn write(self, spec: &ModelSpec, dir: &str) -> Authored {
        match self {
            Cut::Decode { ctx } => author::decode_step(spec, dir, ctx, 1, 1),
            Cut::Prefill { rows, ctx } => author::prefill(spec, dir, ctx, &vec![1; rows], 1),
            Cut::Batch { rows, ctx } => author::decode_batch(spec, dir, ctx, &vec![(1, 1); rows]),
            Cut::Forward => author::forward(spec, dir),
            Cut::ForwardLayer { layer } => author::layer(spec, layer, dir),
            Cut::ForwardHead => author::head(spec, dir),
            Cut::DecodeEmbed => author::decode_embed(spec, dir, 1),
            Cut::DecodeLayer { layer, ctx } => author::decode_layer(spec, layer, dir, ctx, 1),
            Cut::DecodeHead => author::decode_head(spec, dir),
            Cut::BatchEmbed { rows } => author::decode_batch_embed(spec, dir, &vec![1; rows]),
            Cut::BatchLayer { layer, rows, ctx } => {
                author::stacked_layer(spec, layer, dir, ctx, &vec![1; rows], RowState::PerSequence)
            }
            Cut::BatchHead { rows } => author::decode_batch_head(spec, dir, rows),
            Cut::PrefillLayer { layer, rows, ctx } => {
                author::stacked_layer(spec, layer, dir, ctx, &(1..=rows).collect::<Vec<_>>(), RowState::Shared)
            }
            Cut::PrefillHead { rows } => author::prefill_head(spec, dir, rows),
        }
    }

    /// The program `author` writes for this cut of `spec`'s computation split over `tp` chips, its manifest under `dir`:
    /// a decode step, a batch, or one layer of either. Any other cut, or a width the model does not split over, is
    /// refused.
    pub fn write_split(self, spec: &ModelSpec, dir: &str, tp: usize) -> Result<TpAuthored, NotYet> {
        match self {
            Cut::Decode { ctx } => author::tp_decode_step(spec, dir, ctx, 1, 1, tp),
            Cut::Batch { rows, ctx } => author::tp_decode_batch(spec, dir, ctx, &vec![(1, 1); rows], tp),
            Cut::DecodeLayer { layer, ctx } => author::tp_decode_layer(spec, layer, dir, ctx, 1, tp),
            Cut::BatchLayer { layer, rows, ctx } => author::tp_stacked_layer(spec, layer, dir, ctx, &vec![1; rows], tp),
            _ => Err(NotYet("a cut other than a decode step, a batch or a layer of either, split over chips")),
        }
    }
}

impl fmt::Display for Cut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cut::Decode { ctx } => write!(f, "decode step, ctx {ctx}"),
            Cut::Prefill { rows, ctx } => write!(f, "prefill chunk of {rows} rows, ctx {ctx}"),
            Cut::Batch { rows, ctx } => write!(f, "batch of {rows} sequences, ctx {ctx}"),
            Cut::Forward => write!(f, "forward"),
            Cut::ForwardLayer { layer } => write!(f, "forward layer {layer}"),
            Cut::ForwardHead => write!(f, "forward head"),
            Cut::DecodeEmbed => write!(f, "decode step's embedding"),
            Cut::DecodeLayer { layer, ctx } => write!(f, "decode step's layer {layer}, ctx {ctx}"),
            Cut::DecodeHead => write!(f, "decode step's head"),
            Cut::BatchEmbed { rows } => write!(f, "batch of {rows} sequences' embedding"),
            Cut::BatchLayer { layer, rows, ctx } => write!(f, "batch of {rows} sequences' layer {layer}, ctx {ctx}"),
            Cut::BatchHead { rows } => write!(f, "batch of {rows} sequences' head"),
            Cut::PrefillLayer { layer, rows, ctx } => write!(f, "prefill chunk of {rows} rows' layer {layer}, ctx {ctx}"),
            Cut::PrefillHead { rows } => write!(f, "prefill chunk of {rows} rows' head"),
        }
    }
}

/// The program an artifact was lowered from: the cut, the chips it is split over (1 when it is not), and the SHA-256 of
/// the LOGOS text `author` wrote for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Identity {
    pub cut: Cut,
    pub chips: usize,
    pub sha256: [u8; 32],
}

impl Identity {
    /// The identity of `source`, written as the cut `cut` on one chip.
    pub fn of(cut: Cut, source: &str) -> Identity {
        Identity::of_split(cut, 1, source)
    }

    /// The identity of `source`, written as the cut `cut` split over `chips` chips.
    pub fn of_split(cut: Cut, chips: usize, source: &str) -> Identity {
        Identity { cut, chips, sha256: sha256(source.as_bytes()) }
    }

    /// The digest as 64 lowercase hex digits.
    pub fn hex(&self) -> String {
        self.sha256.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The identity as the one line every artifact records it in: `LOGOS program: <cut>, sha256 <hex>`, the cut said
    /// to be split over its chips when it is.
    pub fn line(&self) -> String {
        match self.chips {
            1 => format!("LOGOS program: {}, sha256 {}", self.cut, self.hex()),
            n => format!("LOGOS program: {} over {n} chips, sha256 {}", self.cut, self.hex()),
        }
    }
}

/// The cut `cut` of `spec`'s program, with its identity.
pub fn written(spec: &ModelSpec, cut: Cut) -> (Authored, Identity) {
    written_in(spec, cut, "")
}

/// The cut `cut` of `spec`'s program with its weight manifest under `dir`, and its identity.
pub fn written_in(spec: &ModelSpec, cut: Cut, dir: &str) -> (Authored, Identity) {
    let authored = cut.write(spec, dir);
    let identity = Identity::of(cut, &authored.0);
    (authored, identity)
}

/// [`Cut::Prefill`] at a run's own inputs rather than its placeholders: the chunk of `toks` (token ids from 1) at
/// positions from `pos0`, its manifest under `dir` — the text a run executes. Its identity is the cut's
/// ([`written`]), whose inputs sit at their placeholders.
pub fn prefill_at(spec: &ModelSpec, dir: &str, ctx: usize, toks: &[usize], pos0: usize) -> Authored {
    author::prefill(spec, dir, ctx, toks, pos0)
}

/// [`Cut::Decode`] at a run's own inputs: token `tok` (from 1) at position `pos`, its manifest under `dir`.
pub fn decode_at(spec: &ModelSpec, dir: &str, ctx: usize, tok: usize, pos: usize) -> Authored {
    author::decode_step(spec, dir, ctx, tok, pos)
}

/// The cut `cut` of `spec`'s program split over `tp` chips with its manifest under `dir`, and its identity.
pub fn written_split(spec: &ModelSpec, cut: Cut, dir: &str, tp: usize) -> Result<(TpAuthored, Identity), NotYet> {
    let authored = cut.write_split(spec, dir, tp)?;
    let identity = Identity::of_split(cut, tp, &authored.0);
    Ok((authored, identity))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/model/program/tests.rs"]
mod tests;
