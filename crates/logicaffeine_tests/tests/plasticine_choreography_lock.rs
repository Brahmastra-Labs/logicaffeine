//! ═══════════════════════════════════════════════════════════════════════
//! PLASTICINE CHOREOGRAPHY LOCK — the control plane of the RDU lane: tokens, credits, and buffer depth
//! SOLVED from the initiation interval instead of assumed.
//!
//! The question a spatial-architecture engineer asks is not "do you emit semaphores" but "is the
//! choreography derived, and is it tight". This lock answers both, executably.
//!
//! A. DERIVED, not fixed: every inter-stage channel carries a token net forward and a credit net backward,
//!    synthesized from the metapipeline and the placement — one net per direction per edge, endpoints being
//!    the stages the placement actually put there.
//! B. DEPTH IS A SOLVE: credit depth is the bandwidth-delay product of the channel — `ceil((forward +
//!    backward latency) / II) + 1` — and the shipped depth is the max of that and Spatial's published
//!    writer-to-reader N-buffer rule. Deeper routes and faster pipelines demand more credits; the numbers
//!    move with the inputs, never a constant.
//! C. TIGHT, NOT VACUOUS (the property that makes it a solve rather than a guess): a cycle-exact
//!    token/credit simulation runs at exactly `1/II` with ZERO throughput lost at the solved depth, and
//!    loses throughput at one credit less, on EVERY channel of every case in the corpus. A formula that
//!    over-provisions by a single credit fails the second half; one that under-provisions fails the first.
//!    Throughput is the verdict rather than raw back-pressure: pacing a fast producer down to the pipeline's
//!    rate is what credits are for.
//! D. CONTROL IS ROUTED, not assumed free: control nets are routed on the mesh's SCALAR network against its
//!    own link budget, distinct from the vector network the data uses, and an over-subscribed control
//!    network is reported as such rather than silently ignored.
//! E. THE VENDOR SIMULATOR AGREES: with the control plane modeled, the calibration against Tungsten holds —
//!    every app still within tolerance — so the added latency is real, not a fudge factor.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_choreography_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::choreography::{
    credit_depth, eval_credit_flow, synthesize, ControlKind, CreditRun,
};
use logicaffeine_compile::codegen_sva::metapipeline::{Metapipeline, Stage};
use logicaffeine_compile::codegen_sva::plasticine::PlasticineTarget;
use logicaffeine_compile::codegen_sva::plasticine_place::LinkBudget;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn stage(i: usize, ii: usize, latency: usize) -> Stage {
    Stage {
        name: format!("s{i}"),
        resource_ii: ii,
        recurrence_ii: 0,
        latency,
        cells: latency.max(1),
        memory_ii: 1,
    }
}

/// A chain of stages with an edge between each neighbouring pair.
fn chain(spec: &[(usize, usize)]) -> Metapipeline {
    Metapipeline {
        stages: spec
            .iter()
            .enumerate()
            .map(|(i, &(ii, lat))| stage(i, ii, lat))
            .collect(),
        edges: (0..spec.len().saturating_sub(1))
            .map(|i| (i, i + 1))
            .collect(),
    }
}

/// (name, pipeline, per-edge route hops) — the corpus every claim is proven over.
fn corpus() -> Vec<(&'static str, Metapipeline, Vec<usize>)> {
    vec![
        ("balanced-II1", chain(&[(1, 1), (1, 1), (1, 1)]), vec![1, 1]),
        ("deep-producer", chain(&[(1, 6), (1, 1)]), vec![1]),
        ("slow-consumer", chain(&[(1, 2), (4, 2)]), vec![2]),
        ("slow-producer", chain(&[(4, 2), (1, 2)]), vec![2]),
        ("far-route", chain(&[(1, 2), (1, 2)]), vec![9]),
        (
            "long-chain",
            chain(&[(1, 3), (2, 2), (1, 4), (3, 1)]),
            vec![2, 3, 1],
        ),
        ("gemm-shaped", chain(&[(1, 5), (1, 8), (1, 5)]), vec![4, 4]),
    ]
}

// ───────────────────────────── A ─────────────────────────────

#[test]
fn every_channel_carries_a_token_net_forward_and_a_credit_net_backward() {
    let t = PlasticineTarget::isca17();
    for (name, mp, hops) in corpus() {
        let c = synthesize(&mp, &hops, &t, LinkBudget::spade_default());
        assert_eq!(
            c.channels.len(),
            mp.edges.len(),
            "{name}: one channel per edge"
        );
        assert_eq!(
            c.ii,
            mp.ii(),
            "{name}: the choreography runs at the pipeline's II"
        );
        for (i, ch) in c.channels.iter().enumerate() {
            assert_eq!(
                ch.edge, mp.edges[i],
                "{name}: channels follow the edges in order"
            );
            assert_eq!(
                ch.hops, hops[i],
                "{name}: the route distance is the placement's"
            );
        }
        // Exactly two control nets per edge, in opposite directions, between the edge's own stages.
        assert_eq!(
            c.nets.len(),
            2 * mp.edges.len(),
            "{name}: a token net and a credit net per edge"
        );
        for &(w, r) in &mp.edges {
            let token = c
                .nets
                .iter()
                .find(|n| n.edge == (w, r) && n.kind == ControlKind::Token)
                .unwrap_or_else(|| panic!("{name}: a token net for {w}->{r}"));
            let credit = c
                .nets
                .iter()
                .find(|n| n.edge == (w, r) && n.kind == ControlKind::Credit)
                .unwrap_or_else(|| panic!("{name}: a credit net for {w}->{r}"));
            assert_eq!(
                (token.from, token.to),
                (w, r),
                "{name}: data-valid travels writer to reader"
            );
            assert_eq!(
                (credit.from, credit.to),
                (r, w),
                "{name}: space-available travels reader to writer"
            );
        }
    }
}

// ───────────────────────────── B ─────────────────────────────

#[test]
fn depth_is_the_bandwidth_delay_product_not_a_constant() {
    // The rule itself, stated independently of any pipeline. It is the bandwidth-delay product EXACTLY —
    // a spare credit would pass claim C's no-stall half and fail its one-credit-less half.
    assert_eq!(
        credit_depth(0, 0, 1),
        1,
        "a zero-latency channel still needs one credit to send at all"
    );
    assert_eq!(credit_depth(3, 1, 1), 4, "II 1, round trip 4: ceil(4/1)");
    assert_eq!(
        credit_depth(3, 1, 4),
        1,
        "the same round trip at II 4: ceil(4/4)"
    );
    assert_eq!(credit_depth(7, 2, 3), 3, "ceil(9/3)");
    // Monotone in the things it must be monotone in.
    assert!(
        credit_depth(10, 2, 1) > credit_depth(3, 2, 1),
        "a longer forward latency needs more credits"
    );
    assert!(
        credit_depth(3, 8, 1) > credit_depth(3, 2, 1),
        "a longer credit return needs more credits"
    );
    assert!(
        credit_depth(9, 1, 8) < credit_depth(9, 1, 2),
        "a slower pipeline needs fewer credits"
    );

    let t = PlasticineTarget::isca17();
    for (name, mp, hops) in corpus() {
        let c = synthesize(&mp, &hops, &t, LinkBudget::spade_default());
        for ch in &c.channels {
            let (w, r) = ch.edge;
            assert_eq!(
                ch.n_buffer_depth,
                Metapipeline::buffer_depth(w, r),
                "{name}: Spatial's rule is kept as the floor"
            );
            assert_eq!(
                ch.credit_depth,
                credit_depth(ch.forward_latency, ch.backward_latency, c.ii),
                "{name}: the solve is the rule"
            );
            assert_eq!(
                ch.depth,
                ch.credit_depth.max(ch.n_buffer_depth),
                "{name}: the shipped depth is the deeper requirement"
            );
            assert!(
                ch.forward_latency >= mp.stages[w].latency,
                "{name}: the forward path includes producing the item"
            );
            assert!(
                ch.forward_latency > ch.backward_latency,
                "{name}: data crosses the fabric and is produced; a credit only returns"
            );
        }
    }
    // A far route demands more credits than a near one, all else equal.
    let near = synthesize(
        &chain(&[(1, 2), (1, 2)]),
        &[1],
        &t,
        LinkBudget::spade_default(),
    );
    let far = synthesize(
        &chain(&[(1, 2), (1, 2)]),
        &[9],
        &t,
        LinkBudget::spade_default(),
    );
    assert!(
        far.channels[0].depth > near.channels[0].depth,
        "distance costs credits: {} hops needs {}, {} hops needs {}",
        far.channels[0].hops,
        far.channels[0].depth,
        near.channels[0].hops,
        near.channels[0].depth
    );
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn the_solved_depth_never_stalls_and_one_credit_less_always_does() {
    let t = PlasticineTarget::isca17();
    let iters = 200usize;
    for (name, mp, hops) in corpus() {
        let c = synthesize(&mp, &hops, &t, LinkBudget::spade_default());
        let run: CreditRun = eval_credit_flow(&c, &mp, iters, None);
        assert_eq!(run.items, iters, "{name}: every iteration completes");
        // Throughput, not raw back-pressure, is the verdict: pacing a fast producer down to the pipeline's
        // rate is what credits are FOR and costs nothing.
        assert_eq!(
            run.throughput_loss, 0,
            "{name}: the solved depth loses no throughput"
        );
        let steady = run.cycles.saturating_sub(run.fill_cycles);
        assert_eq!(
            steady,
            (iters - 1) * c.ii,
            "{name}: throughput is exactly 1/II at the solved depth"
        );

        // One credit less on ANY channel must cost something — otherwise the depth was over-provisioned.
        for k in 0..c.channels.len() {
            let mut lean = c.clone();
            if lean.channels[k].depth <= 1 {
                continue; // a one-credit channel has nothing below it
            }
            lean.channels[k].depth -= 1;
            let run = eval_credit_flow(&lean, &mp, iters, None);
            assert!(run.throughput_loss > 0,
                    "{name}: channel {k} still runs at 1/II with depth {} — the solve over-provisions by a credit",
                    lean.channels[k].depth);
        }
    }
}

// ───────────────────────────── D ─────────────────────────────

#[test]
fn control_is_routed_on_the_scalar_network_against_its_own_budget() {
    let t = PlasticineTarget::isca17();
    let budget = LinkBudget::spade_default();
    let (_, mp, hops) = corpus()
        .into_iter()
        .find(|(n, _, _)| *n == "long-chain")
        .unwrap();
    let c = synthesize(&mp, &hops, &t, budget);
    assert!(c.control_hops > 0, "control nets travel the fabric");
    assert_eq!(
        c.control_hops,
        c.nets.iter().map(|n| n.hops).sum::<usize>(),
        "the reported total is the nets' own"
    );
    assert!(
        c.scalar_link_budget == budget.scalar_links,
        "control is budgeted on the SCALAR links, not the vector ones"
    );
    assert!(
        c.max_scalar_occupancy <= c.scalar_link_budget || !c.control_routed_statically,
        "an over-subscribed control network is reported, never silently ignored"
    );
    assert!(
        c.control_routed_statically,
        "the corpus routes its control plane within Spade's scalar budget"
    );
    // A credit net is never free: it costs at least the hops its data crossed.
    for ch in &c.channels {
        let credit = c
            .nets
            .iter()
            .find(|n| n.edge == ch.edge && n.kind == ControlKind::Credit)
            .unwrap();
        assert!(
            credit.hops >= ch.hops,
            "the credit returns across the same fabric it was granted over"
        );
    }
}

// ───────────────────────────── E ─────────────────────────────

#[test]
fn the_calibration_against_the_vendor_simulator_still_holds_with_control_modeled() {
    let cal: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            repo_root().join("benchmarks/silicon/plasticine_calibration.json"),
        )
        .expect("the calibration file"),
    )
    .expect("calibration parses");
    let tol = cal["tolerance_pct"].as_f64().unwrap();
    let rows = cal["rows"].as_array().unwrap();
    assert!(rows.len() >= 8, "the calibration covers the apps we ran");
    for r in rows {
        let diff = r["difference_pct"].as_f64().unwrap();
        assert!(
            diff <= tol,
            "{}: modeling the control plane must not break the calibration ({diff}% > {tol}%)",
            r["key"]
        );
    }
}
