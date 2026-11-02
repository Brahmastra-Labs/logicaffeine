//! **Operand sharing** — reader fusion on the RDU, decided from the cost model.
//!
//! Unroll a tiled GEMM `par` ways and every instance wants the same operand panels. There are two ways to
//! feed them and the compiler must pick one:
//!
//! * **Replicated** — each instance streams its own copy from DRAM. `consumers` times the memory traffic,
//!   nothing on the fabric.
//! * **Broadcast** — one instance streams it and fans the panel out to the others. One copy from memory, plus
//!   a fan-out that occupies the vector network for as long as the data takes to cross it.
//!
//! Which is cheaper is a property of the shape, the mesh and the memory, never a constant: a large panel
//! shared by many instances over a short hop is obviously worth broadcasting, and a small one over a long hop
//! obviously is not, and the interesting cases are in between. So [`share_operand`] compares the DRAM cycles
//! a broadcast would save against the fabric cycles it would add, and returns the cheaper arrangement with
//! both numbers attached — the same shape of decision the Tensix lane makes for reader fusion, and with the
//! same discipline: the function takes the machine and the shape and nothing else, so there is no way to tell
//! the compiler the answer. `plasticine_fusion_lock` holds that both outcomes are reachable by moving only
//! the costs, and that the roofline moves exactly the bytes the decision chose.

use super::plasticine::PlasticineTarget;

/// How an operand reaches the instances that want it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Sharing {
    /// Every consumer streams its own copy from memory.
    Replicated,
    /// One consumer streams it and fans it out across the fabric.
    Broadcast,
}

impl Sharing {
    pub fn label(&self) -> &'static str {
        match self {
            Sharing::Replicated => "replicated",
            Sharing::Broadcast => "broadcast",
        }
    }
}

/// One operand panel and the instances that want it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperandStream {
    pub name: String,
    /// Bytes in one copy of the panel.
    pub bytes: usize,
    /// Instances that need it.
    pub consumers: usize,
    /// Mesh hops a fan-out would cross to reach them.
    pub broadcast_hops: usize,
}

/// The decision, with the two costs that produced it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct SharingDecision {
    pub sharing: Sharing,
    /// DRAM cycles broadcasting would save: the `consumers - 1` copies it does not fetch.
    pub dram_cycles_saved: usize,
    /// Fabric cycles it would add: the panel crossing `broadcast_hops` links, one word per link per cycle.
    pub fabric_cycles_added: usize,
    /// Copies actually fetched from memory under the chosen arrangement.
    pub copies: usize,
    pub why: String,
}

/// Cycles to move `bytes` across `t`'s memory channels.
fn dram_cycles(bytes: usize, t: &PlasticineTarget) -> usize {
    (bytes as f64 * t.clock_ghz / t.dram.gbps).ceil() as usize
}

/// Choose how `stream` reaches its consumers on `t`. Takes the machine and the shape, and nothing else.
pub fn share_operand(stream: &OperandStream, t: &PlasticineTarget) -> SharingDecision {
    let consumers = stream.consumers.max(1);
    // What broadcasting saves: the copies it does not fetch.
    let saved = dram_cycles(stream.bytes * (consumers - 1), t);
    // What it costs: the panel crossing the fan-out. The links are REGISTERED, so the panel streams — the
    // first word arrives after one cycle per hop, and every word after it follows one cycle behind. `W`
    // words over `H` hops therefore occupy the net for `W + H - 1` cycles, the fill plus the stream, not
    // `W * H`.
    //
    // This read `words * broadcast_hops`, which is the store-and-forward cost: every word paying every hop
    // in turn, as if each switch buffered the whole panel before passing it on. That over-charges a
    // broadcast by about a factor of the distance and biases this decision toward replication — on a
    // 640-cubed A panel it charged ~256,000 cycles where the honest figure is ~25,600, which was enough to
    // flip the wide-mesh preset into fetching one shared operand up to 80 times from DRAM.
    //
    // `choreography.rs` already models the fabric this way (`forward_latency = stage.latency + hops`), and
    // the credit-depth solve's two-sided tightness proof rests on it. Two passes in one compiler cannot
    // disagree about whether a fabric pipelines.
    let word_bytes = 4 * t.pcu_lanes.max(1);
    let words = stream.bytes.div_ceil(word_bytes.max(1));
    let added = words + stream.broadcast_hops.saturating_sub(1);
    let broadcast = consumers > 1 && saved > added;
    let sharing = if broadcast {
        Sharing::Broadcast
    } else {
        Sharing::Replicated
    };
    let why = if consumers <= 1 {
        "one consumer: nothing to share".to_string()
    } else if broadcast {
        format!("broadcasting saves {saved} DRAM cycles and costs {added} on the fabric")
    } else {
        format!("a fan-out would cost {added} fabric cycles to save {saved} DRAM cycles")
    };
    SharingDecision {
        sharing,
        dram_cycles_saved: saved,
        fabric_cycles_added: added,
        copies: if broadcast { 1 } else { consumers },
        why,
    }
}
