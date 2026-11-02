//! **The control plane of a spatial pipeline**: the tokens and credits that make a metapipeline run, and the
//! buffer depth that follows from them.
//!
//! A dataflow stage does not simply "have a buffer of depth 2". A channel between a producer and a consumer
//! is a credit loop: the producer spends a credit to send an item, the item crosses the fabric, the consumer
//! reads it and returns the credit, and the credit crosses back. While that loop is in flight the producer
//! may only keep sending if it holds further credits. So the depth a channel needs to sustain one item every
//! `II` cycles is the loop's bandwidth-delay product,
//!
//! ```text
//!     credits = ceil((forward latency + backward latency) / II)
//! ```
//!
//! and nothing else: fewer stalls the producer, more is dead scratchpad. [`credit_depth`] is that solve,
//! [`synthesize`] applies it per channel from the pipeline and its placement, and [`eval_credit_flow`] is the
//! executable meaning — a cycle-exact token/credit simulation whose steady state runs at exactly `1/II` when
//! the depth is right and stalls the moment a single credit is taken away. That two-sided property is what
//! separates a solve from a guess, and `plasticine_choreography_lock` holds both sides over a corpus.
//!
//! The shipped depth is the deeper of this solve and Spatial's published writer-to-reader N-buffer rule
//! (Koeplinger et al., PLDI 2018: a value written in stage `i` and read in stage `j` must survive `j - i + 1`
//! stage-times), because a channel must satisfy both: enough credits to never stall, and enough buffers to
//! keep every live value alive.
//!
//! Control is not free and not invisible. Every channel carries two nets — data-valid forward, space-available
//! backward — and they are budgeted on the mesh's SCALAR network, separately from the vector network the data
//! itself crosses, against the PCU's own control ports. An over-subscribed control plane is reported, never
//! silently ignored. MODELED: these are the model's cycles, never a Plasticine simulation.

use super::metapipeline::Metapipeline;
use super::plasticine::PlasticineTarget;
use super::plasticine_place::LinkBudget;

/// Which way a control net runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ControlKind {
    /// Data-valid, writer to reader: "an item is here".
    Token,
    /// Space-available, reader to writer: "a slot is free again".
    Credit,
}

/// One control net of a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ControlNet {
    /// The channel it belongs to, always `(writer, reader)`.
    pub edge: (usize, usize),
    pub kind: ControlKind,
    /// The stage it leaves and the stage it reaches (reversed from `edge` for a credit).
    pub from: usize,
    pub to: usize,
    /// Mesh hops the net crosses.
    pub hops: usize,
}

/// A credit-controlled channel between two stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Channel {
    pub edge: (usize, usize),
    /// Mesh hops between the two stages, from the placement.
    pub hops: usize,
    /// Producing the item and carrying it to the consumer.
    pub forward_latency: usize,
    /// Carrying the freed credit back.
    pub backward_latency: usize,
    /// Spatial's writer-to-reader N-buffer rule — the floor.
    pub n_buffer_depth: usize,
    /// The bandwidth-delay solve — what it takes never to stall.
    pub credit_depth: usize,
    /// What the channel ships: the deeper requirement.
    pub depth: usize,
}

/// The synthesized control plane of a metapipeline.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Choreography {
    pub channels: Vec<Channel>,
    pub nets: Vec<ControlNet>,
    /// The pipeline's initiation interval — the rate the credits are sized for.
    pub ii: usize,
    /// Hops summed over every control net.
    pub control_hops: usize,
    /// Most control nets incident on any one stage (its control ports must carry them).
    pub max_scalar_occupancy: usize,
    /// The scalar links a switch provides (Spade `slink`).
    pub scalar_link_budget: usize,
    /// Whether the control plane fits that budget.
    pub control_routed_statically: bool,
}

/// Credits a channel needs to sustain one item every `ii` cycles: the loop's bandwidth-delay product. A
/// channel always holds at least one credit — without one nothing can be sent at all.
pub fn credit_depth(forward_latency: usize, backward_latency: usize, ii: usize) -> usize {
    let round_trip = forward_latency + backward_latency;
    let ii = ii.max(1);
    round_trip.div_ceil(ii).max(1)
}

/// Synthesize the control plane of `mp` given the mesh distance of each edge (`hops[i]` for `mp.edges[i]`).
/// Forward latency is producing the item plus carrying it; backward latency is the credit's return trip.
pub fn synthesize(
    mp: &Metapipeline,
    hops: &[usize],
    t: &PlasticineTarget,
    budget: LinkBudget,
) -> Choreography {
    assert_eq!(hops.len(), mp.edges.len(), "one route distance per edge");
    let ii = mp.ii();
    let mut channels = Vec::with_capacity(mp.edges.len());
    let mut nets = Vec::with_capacity(2 * mp.edges.len());
    for (i, &(w, r)) in mp.edges.iter().enumerate() {
        let h = hops[i];
        let forward_latency = mp.stages[w].latency + h;
        let backward_latency = h;
        let n_buffer_depth = Metapipeline::buffer_depth(w, r);
        let credits = credit_depth(forward_latency, backward_latency, ii);
        channels.push(Channel {
            edge: (w, r),
            hops: h,
            forward_latency,
            backward_latency,
            n_buffer_depth,
            credit_depth: credits,
            depth: credits.max(n_buffer_depth),
        });
        nets.push(ControlNet {
            edge: (w, r),
            kind: ControlKind::Token,
            from: w,
            to: r,
            hops: h,
        });
        nets.push(ControlNet {
            edge: (w, r),
            kind: ControlKind::Credit,
            from: r,
            to: w,
            hops: h,
        });
    }
    let mut incident = vec![0usize; mp.stages.len()];
    for n in &nets {
        incident[n.from] += 1;
        incident[n.to] += 1;
    }
    let max_scalar_occupancy = incident.iter().copied().max().unwrap_or(0);
    // The PCU's own control ports bound the plane as well as the switch's scalar links; a descriptor that
    // declares control ports takes the tighter of the two.
    let ports = t
        .pcu_control_io
        .map(|(i, o)| i.min(o))
        .unwrap_or(usize::MAX);
    let scalar_link_budget = budget.scalar_links;
    Choreography {
        channels,
        nets,
        ii,
        control_hops: hops.iter().sum::<usize>() * 2,
        max_scalar_occupancy,
        scalar_link_budget,
        control_routed_statically: max_scalar_occupancy <= scalar_link_budget.min(ports),
    }
}

/// What a credit-flow run did.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct CreditRun {
    /// Completion cycle of the last iteration.
    pub cycles: usize,
    /// Completion cycle of the FIRST iteration — the pipeline's fill.
    pub fill_cycles: usize,
    /// Cycles lost against the ideal steady state `(iters - 1) * II`. **This is the number that matters.**
    /// Back-pressure that merely paces a fast producer down to the pipeline's rate costs nothing and is not
    /// counted here; only credits that hold the pipeline below `1/II` are a loss.
    pub throughput_loss: usize,
    /// Diagnostic: issues where a credit was the binding constraint, healthy pacing included.
    pub credit_stalls: usize,
    pub items: usize,
}

/// Run the pipeline as a cycle-exact token/credit machine for `iters` iterations.
///
/// Stage `s` issues iteration `t` at the latest of: its own initiation interval since its last issue; the
/// arrival of every input token (`issue(w, t) + forward`); and a free credit on every output channel — which,
/// with `depth` credits outstanding, is the return of the credit spent on iteration `t - depth`
/// (`issue(r, t - depth) + backward`). Stages are visited in edge order, a topological order of the
/// metapipelines this models (writer < reader).
///
/// The verdict is [`CreditRun::throughput_loss`], not the raw stall count: a producer whose own interval is
/// shorter than the pipeline's is *supposed* to be paced by back-pressure, and that costs nothing. A depth is
/// right when the loss is zero and wrong the moment one credit is removed.
pub fn eval_credit_flow(
    c: &Choreography,
    mp: &Metapipeline,
    iters: usize,
    depth_override: Option<usize>,
) -> CreditRun {
    let n = mp.stages.len();
    if iters == 0 || n == 0 {
        return CreditRun {
            cycles: 0,
            fill_cycles: 0,
            throughput_loss: 0,
            credit_stalls: 0,
            items: 0,
        };
    }
    let depth_of = |k: usize| depth_override.unwrap_or(c.channels[k].depth).max(1);
    // issue[s][t] — the cycle stage `s` began iteration `t` (and consumed its inputs).
    let mut issue = vec![vec![0usize; iters]; n];
    let mut credit_stalls = 0usize;
    for t in 0..iters {
        for s in 0..n {
            let ready_ii = if t == 0 {
                0
            } else {
                issue[s][t - 1] + mp.stages[s].ii()
            };
            let ready_data = c
                .channels
                .iter()
                .filter(|ch| ch.edge.1 == s)
                .map(|ch| issue[ch.edge.0][t] + ch.forward_latency)
                .max()
                .unwrap_or(0);
            let ready_credit = c
                .channels
                .iter()
                .enumerate()
                .filter(|(_, ch)| ch.edge.0 == s)
                .map(|(k, ch)| {
                    let d = depth_of(k);
                    if t < d {
                        0
                    } else {
                        issue[ch.edge.1][t - d] + ch.backward_latency
                    }
                })
                .max()
                .unwrap_or(0);
            let without_credit = ready_ii.max(ready_data);
            if ready_credit > without_credit {
                credit_stalls += 1;
            }
            issue[s][t] = without_credit.max(ready_credit);
        }
    }
    let last = n - 1;
    let done = |t: usize| issue[last][t] + mp.stages[last].latency;
    let (cycles, fill_cycles) = (done(iters - 1), done(0));
    let ideal = (iters - 1) * mp.ii();
    CreditRun {
        cycles,
        fill_cycles,
        throughput_loss: (cycles - fill_cycles).saturating_sub(ideal),
        credit_stalls,
        items: iters,
    }
}
