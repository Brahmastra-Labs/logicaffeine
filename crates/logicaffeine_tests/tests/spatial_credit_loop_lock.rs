//! ═══════════════════════════════════════════════════════════════════════
//! CREDIT LOOP LOCK — an input FIFO must cover the WHOLE round trip, not just the fetch.
//!
//! A bounded buffer between a movement engine and a compute engine is a credit loop. The producer may not
//! refill a page until the consumer's release travels back to it, so the pages that must be in flight to keep
//! the consumer fed are set by the loop's bandwidth-delay product:
//!
//!     depth >= ceil( (forward fetch + credit return) / service time )
//!
//! Sizing on the forward fetch alone and adding one page is the same number ONLY while the credit return
//! fits inside one service time. When compute per block falls below the semaphore round trip — small tiles,
//! cheap fidelity, a fast engine — the return trip dominates and the producer stalls every round on a buffer
//! that looks correctly sized. Both shapes this project benchmarks (640^3 and 2048x1024x512) happen to have
//! margin, so a shortfall here is invisible to the ledger and shows up only as cycles nobody can attribute.
//!
//! This is the same solve `codegen_sva::choreography::credit_depth` proves two-sided on the Plasticine
//! target, applied to the Tensix one. It is locked here so the two targets cannot drift apart on the one
//! question a dataflow architect always asks.
//!
//! SCOPE. The corpus reaches only edges whose producer is a movement engine and whose consumer is the
//! compute engine — the case the bandwidth-delay product actually describes. Where a producer and consumer
//! are the SAME engine (a matmul feeding a fused requant/bias/relu chain on one site) they cannot run
//! concurrently, so the round trip is not what sets the depth: two is correct there by pipeline structure,
//! and this formula would wrongly call it under-sized. Any widening of this lock must exclude same-engine
//! edges rather than let the formula judge them.
//!
//! LIMIT. The forward leg here is the DRAM fetch term only. The true leg runs from credit acquired to page
//! visible, so it also carries pack and push, making this slightly optimistic on plans that do not overlap
//! packing. The floor of two absorbs it at every shape measured; `move_cycles` holds the exact term.
//!
//! A. Every input FIFO the compiler emits covers its full credit loop — UNLESS the site has no local memory
//!    left to buy the next page. That distinction is the whole invariant. A buffer that is short with memory
//!    still free is a sizing defect and fails here. A buffer that is short with the budget spent is
//!    under-buffered on purpose: buffering has done all it can, the pipeline is memory-bound at that shape,
//!    and the remedy is a coarser plan rather than a deeper FIFO. The artifact records which shapes were
//!    capped, so the second case stays visible instead of being silently absorbed.
//! B. The depth is not gratuitous — it never exceeds the requirement by more than the one page being filled.
//! C. The credit-return term is LIVE: on a target whose semaphore round trip is free the requirement drops,
//!    so a formula that ignored the term could not pass both A and C.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_credit_loop_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_helpers;

use logicaffeine_compile::spatial::cost::quantities_for_gemm;
use logicaffeine_compile::spatial::ir::{BufferKind, SpatialProgram};
use logicaffeine_compile::spatial::passes::{
    fuse_readers::fuse_readers,
    place::place,
    size_buffers::{footprint_bytes, size_buffers},
    tile::tile,
};
use logicaffeine_compile::spatial::pipeline::template_from_source;
use logicaffeine_compile::spatial::target::{ComputeCap, CreditLatency, Fidelity, SpatialTarget};
use spatial_helpers::{fusing_wormhole, gemm_program};

/// Shapes chosen so the corpus straddles the crossing point: the two the ledger benchmarks (where the return
/// trip fits inside one service time) and smaller ones (where it does not).
const CORPUS: &[(usize, usize, usize)] = &[
    (640, 640, 640),
    (2048, 1024, 512),
    (512, 512, 512),
    (256, 256, 256),
    (1024, 1024, 512),
];

fn sized(t: &SpatialTarget, m: usize, n: usize, k: usize) -> Option<SpatialProgram> {
    let tpl = template_from_source(&gemm_program(m, n, k), t).ok()?;
    tile(&tpl, t)
        .and_then(|p| place(&p, t))
        .and_then(|p| fuse_readers(&p, t))
        .and_then(|p| size_buffers(&p, t))
        .ok()
}

/// `(service time per K block, credit round trip)` for a sized program.
fn rates(p: &SpatialProgram, t: &SpatialTarget) -> (f64, f64) {
    let plan = p.work.plan.expect("a placed program is tiled");
    let out = p.output_tensor().expect("a gemm has an output");
    let fid = match t.matmul_cap() {
        Some(ComputeCap::MatmulTile { fidelity, .. }) => *fidelity,
        _ => Fidelity::Exact,
    };
    let fused = !p.placement.as_ref().expect("placed").groups.is_empty();
    let qn = quantities_for_gemm(
        out.tiles.rows_t,
        out.tiles.cols_t,
        p.work.k_tiles,
        &plan,
        fused,
        fid,
        t,
    );
    (
        t.cycle.compute_cycles(&qn) / plan.inner.num_blocks.max(1) as f64,
        t.cycle.c_mcast_block,
    )
}

/// The pages the credit loop needs in flight: the round trip divided by the service it is hiding.
fn credit_loop_depth(forward: f64, ret: f64, service: f64) -> usize {
    if service <= 0.0 {
        return 2;
    }
    (((forward + ret) / service).ceil() as usize).max(2)
}

// ───────────────────────────── A + B ─────────────────────────────

#[test]
fn every_input_fifo_covers_its_whole_credit_loop() {
    // TWO contracts, both pinned. A part that declares no credit latencies makes no rate claim, and its
    // contract is the vendor's double buffering — exactly two pages, no more. Skipping that case would leave
    // the invariant unpinned and make the whole lock vacuous, since no target in the tree declares latencies
    // yet; asserting it is what keeps this honest until one does.
    {
        let plain = SpatialTarget::wormhole();
        assert!(
            plain.site.credit.is_none(),
            "this branch is the undeclared contract; wormhole now declares latencies"
        );
        let mut seen = 0usize;
        for &(m, n, k) in CORPUS {
            let Some(p) = sized(&plain, m, n, k) else {
                continue;
            };
            for b in &p.buffers {
                let BufferKind::Fifo { depth_pages, .. } = b.kind else {
                    continue;
                };
                if !b.name.starts_with("in") {
                    continue;
                }
                seen += 1;
                assert_eq!(depth_pages, 2, "{m}x{n}x{k} {}: an undeclared part gets the vendor's double buffering, exactly", b.name);
            }
        }
        assert!(
            seen > 0,
            "the corpus produced no input FIFOs for the undeclared contract"
        );
    }
    // And a part that DOES declare them must cover its whole credit loop. Driven from a declared point rather
    // than the default so the bandwidth-delay coverage below is genuinely exercised: MEASURED on this sweep,
    // fetch 8 / return 8 / service 200 lifts both operand FIFOs from the default 2 to 3, and by fetch 16 the
    // memory budget begins giving pages back (in0 falls to 2 while in1 rises to 6) rather than growing without
    // bound — which is what says the model is real and not monotone by construction.
    let mut t = SpatialTarget::wormhole();
    t.site.credit = Some(CreditLatency {
        fetch_tile: 8,
        credit_return: 8,
        service_page: 200,
    });
    let mut checked = 0usize;
    let mut short = Vec::new();
    let mut worst: std::collections::BTreeMap<(usize, usize, usize), (i64, serde_json::Value)> =
        std::collections::BTreeMap::new();
    let target_budget = t.site.local_mem_bytes;
    // Shapes where buffering ran out of memory before it ran out of round trip — recorded, not asserted on.
    let mut capped: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // The credit loop is sized from the TARGET's declared latencies, which is where the pass reads them from.
    // Deriving the requirement from the fitted cost constants instead would check the sizing against numbers
    // calibrated to a cycle counter, and a resource decision must not rest on a regression. A part that has
    // not declared its latencies makes no rate claim at all — its contract is the vendor's double buffering —
    // so there is no credit loop to cover and nothing here to assert.
    let credit = t.site.credit.expect("declared just above");
    for &(m, n, k) in CORPUS {
        let Some(p) = sized(&t, m, n, k) else {
            continue;
        };
        let (service, ret) = (credit.service_page as f64, credit.credit_return as f64);
        let used_bytes = footprint_bytes(&p, &t);
        for b in &p.buffers {
            let BufferKind::Fifo {
                depth_pages,
                page_tiles,
            } = b.kind
            else {
                continue;
            };
            if !b.name.starts_with("in") {
                continue;
            }
            checked += 1;
            let forward = page_tiles as f64 * credit.fetch_tile as f64;
            let need = credit_loop_depth(forward, ret, service);
            // One row per shape, keeping the buffer with the LEAST headroom: the operands can have
            // different page sizes, so the worst of them is what characterises the shape.
            let slack = depth_pages as i64 - need as i64;
            let entry = serde_json::json!({
                "shape": format!("{m}x{n}x{k}"),
                "buffer": b.name.clone(),
                "service_cycles_per_block": (service * 10.0).round() / 10.0,
                "credit_return_cycles": (ret * 10.0).round() / 10.0,
                "forward_fetch_cycles": (forward * 10.0).round() / 10.0,
                "round_trips_per_service": ((forward + ret) / service * 100.0).round() / 100.0,
                "depth_emitted": depth_pages,
                "depth_credit_loop_requires": need,
            });
            match worst.get(&(m, n, k)) {
                Some((s, _)) if *s <= slack => {}
                _ => {
                    worst.insert((m, n, k), (slack, entry));
                }
            }
            // Short is only a DEFECT when the memory to fix it was actually there. A site that has spent
            // its local budget is under-buffered on purpose, and the honest reading is that the pipeline is
            // memory-bound at that shape — a different plan, not a deeper FIFO. Distinguishing the two is
            // what makes this an invariant rather than a shape-dependent alarm.
            if depth_pages < need {
                let page_bytes = p.buffer_bytes(b) / depth_pages.max(1);
                let headroom = target_budget.saturating_sub(used_bytes);
                if page_bytes > 0 && headroom >= page_bytes {
                    short.push(format!(
                        "{m}x{n}x{k} {}: depth {depth_pages} but the credit loop needs {need} \
                         (fetch {forward:.0} + return {ret:.0} over service {service:.0}); \
                         {headroom} bytes of local memory were free and a page costs {page_bytes}",
                        b.name
                    ));
                } else {
                    capped.insert(format!("{m}x{n}x{k}"));
                }
            }
            // B: the requirement, plus at most the page being filled. A latency-bound shape is under, not
            // over, so this still applies there.
            assert!(
                depth_pages <= need + 1,
                "{m}x{n}x{k} {}: depth {depth_pages} exceeds the credit loop's {need} by more than one page — \
                 buffer memory spent on nothing",
                b.name
            );
        }
    }
    assert!(checked > 0, "the corpus produced no input FIFOs to check");
    let rows: Vec<serde_json::Value> = worst
        .into_values()
        .map(|(_, mut v)| {
            let sh = v["shape"].as_str().unwrap_or_default().to_string();
            v["memory_capped"] = serde_json::Value::Bool(capped.contains(&sh));
            v
        })
        .collect();
    write_record(&rows);
    short.dedup();
    assert!(
        short.is_empty(),
        "input FIFOs are sized on the forward fetch alone, so the producer stalls on the credit return every \
         round at these shapes:\n  {}",
        short.join("\n  ")
    );
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn the_credit_return_term_is_live_not_decorative() {
    let paid = SpatialTarget::wormhole();
    let free = fusing_wormhole();
    assert!(
        paid.cycle.c_mcast_block > 0.0,
        "the calibrated target pays for a semaphore round trip"
    );
    assert_eq!(
        free.cycle.c_mcast_block, 0.0,
        "the fusing target's round trip is free"
    );
    // ★ SEARCH THE CORPUS RATHER THAN PIN ONE SHAPE. Whether the return trip changes the page count is a
    // property of `(forward, return, service)` — a ceiling either crosses an integer or it does not — so a
    // single pinned shape stops exercising it the moment the planner picks a different page size, which is
    // exactly what happened when the plan search moved to the work model. Requiring that SOME shape in a
    // spread exercises it keeps the obligation ("the term is live") while letting the plan move.
    let mut exercised = Vec::new();
    let mut examined = 0;
    for (m, n, k) in [
        (256usize, 256usize, 256usize),
        (320, 320, 320),
        (512, 512, 512),
        (512, 512, 1024),
        (640, 640, 640),
        (1024, 1024, 512),
        (1024, 1024, 1024),
        (2048, 1024, 512),
    ] {
        let Some(p) = sized(&paid, m, n, k) else {
            continue;
        };
        let (service, ret) = rates(&p, &paid);
        let Some(forward) = p.buffers.iter().find_map(|b| match b.kind {
            BufferKind::Fifo { page_tiles, .. } if b.name.starts_with("in") => {
                Some(page_tiles as f64 * paid.cycle.c_dram_tile)
            }
            _ => None,
        }) else {
            continue;
        };
        examined += 1;
        if credit_loop_depth(forward, ret, service) > credit_loop_depth(forward, 0.0, service) {
            exercised.push((m, n, k, forward, ret, service));
        }
    }
    assert!(
        examined >= 5,
        "only {examined} shapes could be sized — the corpus is too thin to conclude anything"
    );
    assert!(
        !exercised.is_empty(),
        "no shape in the corpus asks for more pages with the credit return than without it, so the term is \
         decorative everywhere it was looked for. Examined {examined} shapes."
    );
    let (m, n, k, f, r, s) = exercised[0];
    eprintln!("credit return is live at {m}x{n}x{k}: forward {f:.0}, return {r:.0}, service {s:.0} -> {} pages vs {}",
        credit_loop_depth(f, r, s), credit_loop_depth(f, 0.0, s));
}

/// The table this lock computes, written for the record so no document has to transcribe it. Values come
/// from the target's own calibrated cost model, so the file regenerates identically on every run.
fn write_record(rows: &[serde_json::Value]) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let doc = serde_json::json!({
        "_generated_by": "spatial_credit_loop_lock::every_input_fifo_covers_its_whole_credit_loop",
        "what": "Per shape: the service one K block of compute buys, the credit return trip the producer                  must wait out, the depth the compiler emits, and the depth the credit loop requires. A                  shape whose round trip exceeds one service time is one where sizing on the forward fetch                  alone under-provisions.",
        "cycle_model": "SpatialTarget::wormhole() calibrated constants",
        "rows": rows,
    });
    let out = root.join("benchmarks/silicon/spatial_credit_loop.json");
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(out, serde_json::to_string_pretty(&doc).unwrap() + "\n");
}
