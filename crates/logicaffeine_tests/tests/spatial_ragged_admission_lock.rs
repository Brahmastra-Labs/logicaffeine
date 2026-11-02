//! A ragged grid is chosen only where it costs nothing in total work (ruling A12, 2026-09-27).
//!
//! The fill-the-mesh planner offers grids whose extent does not divide the tile count: the last row or column
//! of sites carries what is left. `Balanced` ranks the busiest site's arithmetic second, so it favours such a
//! grid whenever it spreads the arithmetic thinner — even where the spread moves more bytes in more
//! transactions than an evenly divided plan beside it, a benefit the simulator cannot show. So a ragged plan
//! is admissible only where no evenly divided plan beats it on every predicted total (bytes moved,
//! transactions, compute-engine calls), and `Balanced` ranks the rest as before. The rule is over the even
//! plans only: over every plan it would choose the fewest sites, since a plan on fewer sites moves fewer bytes
//! in total, and refuse the parallelism the objective exists to buy.

use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::passes::tile::{best_candidate_for, candidates, Candidate};
use logicaffeine_compile::spatial::pipeline::template_from_source_with;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::work::Objective;

/// The GEMM the measurement harness compiles, so the plan under test is the plan that ships.
fn gemm_source(m: usize, n: usize, k: usize) -> String {
    format!(
        "## Main\nLet rows be {m}.\nLet cols be {n}.\nLet mid be {k}.\nLet mutable a be a new Seq of Int.\n\
         Let mutable b be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable p be 0.\n\
         While p is less than rows * mid:\n    Push (p % 7) - 3 to a.\n    Set p to p + 1.\nSet p to 0.\n\
         While p is less than mid * cols:\n    Push (p % 11) - 5 to b.\n    Set p to p + 1.\nSet p to 0.\n\
         While p is less than rows * cols:\n    Push 0 to c.\n    Set p to p + 1.\nLet mutable i be 0.\n\
         While i is less than rows:\n    Let mutable k be 0.\n    While k is less than mid:\n        \
         Let mutable j be 0.\n        While j is less than cols:\n            Let idx be i * cols + j + 1.\n            \
         Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.\n            \
         Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nLet mutable checksum be 0.\nLet mutable q be 1.\n\
         While q is at most rows * cols:\n    Set checksum to checksum + item q of c.\n    Set q to q + 1.\nShow checksum.\n"
    )
}

/// The candidates of the `m×n×k` GEMM on `target`, with the output's tile counts.
fn lattice(m: usize, n: usize, k: usize, target: &SpatialTarget) -> (Vec<Candidate>, usize, usize) {
    let template = template_from_source_with(
        &gemm_source(m, n, k),
        target,
        NumericsPolicy::AllowSpillRounding,
    )
    .unwrap_or_else(|e| panic!("{m}x{n}x{k} does not template: {e:?}"));
    let out = template.output_tensor().expect("a GEMM has an output");
    let (mt, nt) = (out.tiles.rows_t, out.tiles.cols_t);
    (candidates(&template, target), mt, nt)
}

fn chosen(m: usize, n: usize, k: usize, target: &SpatialTarget, objective: Objective) -> Candidate {
    let template = template_from_source_with(
        &gemm_source(m, n, k),
        target,
        NumericsPolicy::AllowSpillRounding,
    )
    .unwrap();
    best_candidate_for(&template, target, objective)
        .unwrap_or_else(|| panic!("{m}x{n}x{k}: no plan"))
}

fn is_ragged(c: &Candidate, mt: usize, nt: usize) -> bool {
    mt % c.plan.per_site_m != 0 || nt % c.plan.per_site_n != 0
}

fn totals(c: &Candidate) -> (u64, u64, u64) {
    (c.work.noc_bytes, c.work.noc_txns, c.work.compute_calls)
}

/// `a` is no worse than `b` on every total, and better on one.
fn beats(a: &Candidate, b: &Candidate) -> bool {
    let (x, y) = (totals(a), totals(b));
    x.0 <= y.0 && x.1 <= y.1 && x.2 <= y.2 && x != y
}

#[test]
fn at_640_cubed_the_even_5x5_plan_wins_over_the_ragged_7x5_that_costs_more() {
    let target = SpatialTarget::wormhole_ttsim();
    let c = chosen(640, 640, 640, &target, Objective::Balanced);
    assert_eq!(
        (c.quantities.sites_used, c.plan.per_site_m, c.plan.per_site_n),
        (25, 4, 4),
        "640³ on the Wormhole twin chooses the 5×5 grid of 4×4 blocks, not a ragged grid that moves more bytes in \
         more transactions: {c:?}"
    );
}

#[test]
fn a_ragged_plan_is_never_chosen_where_an_even_plan_beats_it_on_every_total() {
    let shapes = [
        (512, 640, 512),
        (640, 640, 640),
        (768, 1280, 768),
        (1280, 2048, 1280),
        (2048, 2048, 640),
        (4096, 256, 256),
    ];
    for target in [SpatialTarget::wormhole_ttsim(), SpatialTarget::blackhole()] {
        for &(m, n, k) in &shapes {
            let (cands, mt, nt) = lattice(m, n, k, &target);
            let c = chosen(m, n, k, &target, Objective::Balanced);
            if !is_ragged(&c, mt, nt) {
                continue;
            }
            let better = cands.iter().find(|o| !is_ragged(o, mt, nt) && beats(o, &c));
            assert!(
                better.is_none(),
                "{m}x{n}x{k}: the chosen ragged plan ({} sites, totals {:?}) is beaten on every total by an evenly \
                 divided plan ({} sites, totals {:?})",
                c.quantities.sites_used,
                totals(&c),
                better.map(|o| o.quantities.sites_used).unwrap_or(0),
                better.map(totals).unwrap_or_default()
            );
        }
    }
}

#[test]
fn the_rule_keeps_the_mesh_spread_where_the_grid_divides() {
    let target = SpatialTarget::wormhole_ttsim();
    for (m, n, k) in [(512, 512, 512), (2048, 2048, 2048), (1280, 2048, 1280)] {
        let c = chosen(m, n, k, &target, Objective::Balanced);
        assert_eq!(
            c.quantities.sites_used, 64,
            "{m}x{n}x{k} keeps every site of the mesh: {c:?}"
        );
    }
}

#[test]
fn only_balanced_admits_by_the_rule() {
    let target = SpatialTarget::wormhole_ttsim();
    let c = chosen(512, 512, 512, &target, Objective::Traffic);
    assert_eq!(
        c.quantities.sites_used, 1,
        "Traffic still reads each operand tile once, on one site: {c:?}"
    );
}
