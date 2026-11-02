//! The engine's memory traffic on the card's DDR4 channels: what the controller model
//! ([`crate::codegen_sva::ddr4_jedec`]) makes of each kind of stream a pass issues, at the card's own timing.
//!
//! A pass streams three kinds of lines through a channel. The weights are one long read of the channel's share of the
//! weight image. A Gated-DeltaNet memory, rewritten every pass, is read and written back in place a chunk at a time,
//! the chunk being what the recurrence unit holds between its read and its write. An attention block's cache is one
//! long read of a sequence's positions and an append of the new one after them. [`ddr_ledger`] prices each on one
//! channel, the state and cache bytes from the decode pricer's own accounting of Qwen3.8-27B's header (an f16 memory
//! and conv ring, an int8 cache), at both of the engine's beats, four sequences back to back so the turns between
//! them are paid; every trace is held to JESD79-4 as it is issued.

use std::path::Path;

use serde::Serialize;

use crate::codegen_sva::ddr4_jedec::{measure, Ddr4Facts, Ddr4Grade, Ddr4Timing, D5005_EMIF_IP, LOOKAHEAD};
use crate::codegen_sva::dram::StreamDesc;
use crate::spatial::d5005::{Beat, DDR_CHANNELS};
use crate::spatial::decode::{DecodeStep, Deployment, GgufHeader, StageKind};

/// The checkpoint whose state and cache the ledger streams.
pub const HEADER: &str = "qwen3.8-27b-q4_k_m";
/// Lines of the weight stream priced: long enough to pay its refreshes as a whole pass does.
pub const WEIGHT_LINES: u64 = 1 << 16;
/// Lines the recurrence unit reads before it writes them back.
pub const STATE_CHUNKS: [u64; 4] = [8, 32, 128, 512];
/// Positions a sequence's cache holds when it is read.
pub const CACHE_POSITIONS: [u64; 3] = [256, 2048, 8192];
/// Sequences each state and cache row streams back to back.
pub const SEQUENCES: u64 = 4;

/// One stream priced on one channel.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DdrRow {
    pub pattern: String,
    pub beat: Beat,
    pub line_bytes: u64,
    pub facts: Ddr4Facts,
    pub efficiency: f64,
    /// Controller clocks the stream takes.
    pub controller_ck: u64,
    pub channel_bytes_per_s: f64,
    /// The four channels each streaming alike.
    pub card_bytes_per_s: f64,
}

/// The ledger: the channel's clocked rules, the controller model's queue, the bytes the streams are sized from, and
/// every stream's price.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DdrLedger {
    pub source: &'static str,
    pub timing: Ddr4Timing,
    pub lookahead: usize,
    pub header: &'static str,
    /// Bytes of one sequence's Gated-DeltaNet memory and conv ring in one block, f16.
    pub state_bytes: u64,
    /// Bytes one position of one attention block's int8 cache holds.
    pub position_bytes: u64,
    pub rows: Vec<DdrRow>,
}

fn line_bytes(beat: Beat, t: &Ddr4Timing) -> u64 {
    match beat {
        Beat::Data64 => 64,
        Beat::Full72 => t.burst_bits() / 8,
    }
}

fn run(base: u64, bytes: u64, write: bool) -> StreamDesc {
    StreamDesc { base, elem_bytes: bytes, stride_bytes: bytes, count: 1, write }
}

/// One sequence's state and one position of its cache, from the decode pricer's stages of `h` with an f16 state and
/// an int8 cache.
pub fn stream_bytes(h: &GgufHeader) -> Result<(u64, u64), String> {
    let d = Deployment { kv_bytes: 1.0, state_bytes: 2.0, ..Deployment::as_stored(1) };
    let step = DecodeStep::of(h, &d).map_err(|e| format!("{e:?}"))?;
    let state = step.stages.iter().find(|s| s.kind == StageKind::Recurrence).ok_or("no recurrent block")?.state_read;
    let position = step.stages.iter().find(|s| s.kind == StageKind::Attention).ok_or("no attention block")?.state_write;
    Ok((state as u64, position as u64))
}

/// Every stream of a pass priced on one D5005 channel.
pub fn ddr_ledger(decode_dir: &Path) -> Result<DdrLedger, String> {
    let h = GgufHeader::read_ledger(&decode_dir.join(format!("headers/{HEADER}.json")))?;
    let (state_bytes, position_bytes) = stream_bytes(&h)?;
    let t = Ddr4Grade::D5005_EMIF.at();
    let mut rows = Vec::new();
    for beat in [Beat::Data64, Beat::Full72] {
        let lb = line_bytes(beat, &t);
        let mut price = |pattern: String, streams: Vec<StreamDesc>| -> Result<(), String> {
            let facts = measure(&t, lb, &streams).map_err(|v| format!("{pattern}: {v}"))?;
            let channel = facts.bytes_per_s(&t, lb);
            rows.push(DdrRow {
                pattern,
                beat,
                line_bytes: lb,
                facts,
                efficiency: facts.efficiency(),
                controller_ck: t.controller_ck(facts.end),
                channel_bytes_per_s: channel,
                card_bytes_per_s: channel * DDR_CHANNELS as f64,
            });
            Ok(())
        };
        price(format!("weights: one read of {WEIGHT_LINES} lines"), vec![run(0, WEIGHT_LINES * lb, false)])?;
        let state_lines = state_bytes.div_ceil(lb);
        for chunk in STATE_CHUNKS {
            let mut streams = Vec::new();
            for s in 0..SEQUENCES {
                let base = s * state_lines;
                let mut at = 0;
                while at < state_lines {
                    let n = chunk.min(state_lines - at);
                    streams.push(run((base + at) * lb, n * lb, false));
                    streams.push(run((base + at) * lb, n * lb, true));
                    at += n;
                }
            }
            price(format!("Gated-DeltaNet state: {SEQUENCES} memories of {state_lines} lines, read and written back {chunk} lines at a time"), streams)?;
        }
        for positions in CACHE_POSITIONS {
            let region = ((positions + 1) * position_bytes).div_ceil(lb);
            let mut streams = Vec::new();
            for s in 0..SEQUENCES {
                let base = s * region * lb;
                streams.push(run(base, positions * position_bytes, false));
                streams.push(run(base + positions * position_bytes, position_bytes, true));
            }
            price(format!("attention cache: {SEQUENCES} sequences of {positions} positions read, one appended"), streams)?;
        }
    }
    Ok(DdrLedger { source: D5005_EMIF_IP, timing: t, lookahead: LOOKAHEAD, header: HEADER, state_bytes, position_bytes, rows })
}
