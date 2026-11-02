//! A BOUNDARY SITE'S KERNEL MOVES ITS OWN PAGE.
//!
//! On a grid whose extents do not divide the tile count, `place` gives a boundary site a narrower operand
//! page — `h · ibw` tiles of A where the interior's is `pm · ibw` — and the IR prices every multicast by that
//! site's page (`work::work_of`, which `spatial_work_model_lock` holds to the closed form and the closed form
//! to the machine). A printed Tensix kernel counts pages in literals, so the payload it multicasts must be the
//! page of the sites it runs on. Printed from the TEMPLATE's page instead, a boundary site multicasts a nominal
//! panel whose last rows do not exist: `kt` extra tiles per ragged axis.
//!
//! The fixture is 704³ on the ttsim twin, whose chosen grid is ragged: 22 N-tiles over 8 columns of 3, so the
//! last column's B page is one tile wide. A planner that stopped choosing a ragged grid here would leave every
//! page nominal, so the lock first asserts the plan is ragged and a boundary page is multicast.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tensix_boundary_page_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;

use logicaffeine_compile::spatial::ir::{ChoreoOp, NumericsPolicy, SpatialProgram, TaggedOp};
use logicaffeine_compile::spatial::lower::tensix::TensixBackend;
use logicaffeine_compile::spatial::pipeline::compile_to_spatial_with;
use logicaffeine_compile::spatial::programs::gemm_program;
use logicaffeine_compile::spatial::target::SpatialTarget;

/// Every multicast payload the IR's engine programs send, in tiles: `pages ·` the sending site's page.
fn ir_multicast_payloads(p: &SpatialProgram) -> BTreeSet<usize> {
    fn walk(p: &SpatialProgram, ops: &[TaggedOp], out: &mut BTreeSet<usize>) {
        for t in ops {
            match &t.op {
                ChoreoOp::McastSend { buf, pages, .. } => {
                    out.insert(pages * p.buffer(*buf).kind.page_tiles());
                }
                ChoreoOp::Loop { body, .. } => walk(p, body, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    for prog in &p.schedule.as_ref().expect("a scheduled program").programs {
        walk(p, &prog.ops, &mut out);
    }
    out
}

/// Every multicast payload the printed kernels issue, in tiles — the `N` of
/// `noc_async_write_multicast(src, get_noc_multicast_addr(…), Nu * TILE_BYTES, …)`.
fn printed_multicast_payloads(sources: &[String]) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for src in sources {
        for line in src
            .lines()
            .filter(|l| l.contains("noc_async_write_multicast("))
        {
            let after_addr = line
                .split("), ")
                .nth(1)
                .expect("a multicast prints its address, then its size");
            let tiles = after_addr
                .split("u * ")
                .next()
                .expect("the size is `<tiles>u * <tile bytes>`");
            out.insert(
                tiles
                    .trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("a literal tile count in: {line}")),
            );
        }
    }
    out
}

const DIM: usize = 704;
const TILE: usize = 32;

fn ragged_704_cubed() -> SpatialProgram {
    let target = SpatialTarget::wormhole_ttsim();
    compile_to_spatial_with(
        &gemm_program(DIM, DIM, DIM),
        &target,
        NumericsPolicy::AllowSpillRounding,
    )
    .expect("704³ compiles")
}

#[test]
fn every_multicast_moves_the_page_of_the_site_that_sends_it() {
    let p = ragged_704_cubed();
    let ir = ir_multicast_payloads(&p);
    let plan = p.work.plan.as_ref().expect("a planned program");
    let tiles = DIM / TILE;
    assert!(
        plan.grid.0 * plan.per_site_m != tiles || plan.grid.1 * plan.per_site_n != tiles,
        "704³ on the ttsim twin no longer chooses a ragged grid ({plan:?}) — pick a shape whose chosen grid does not \
         divide its tile count, or this lock tests nothing"
    );
    let interior = [
        plan.per_site_m * plan.block.ibw,
        plan.per_site_n * plan.block.ibw,
    ];
    assert!(
        ir.iter().any(|t| interior.iter().all(|page| t % page != 0)),
        "no multicast carries a boundary site's page (payloads {ir:?}, interior pages {interior:?}) — the ragged \
         grid's boundary sites send nothing this lock can check"
    );
    let kernels = TensixBackend::new(SpatialTarget::wormhole_ttsim())
        .printed_kernels(&p)
        .expect("the program prints");
    let printed =
        printed_multicast_payloads(&kernels.iter().map(|k| k.source.clone()).collect::<Vec<_>>());
    assert_eq!(
        printed, ir,
        "the printed kernels multicast {printed:?} tiles per send where the program's sites send {ir:?} — a kernel \
         counted a page that is not the page of the sites it runs on"
    );
}

#[test]
fn the_payload_parser_reads_what_the_printer_writes() {
    let line = "noc_async_write_multicast(wp_cb0_start, get_noc_multicast_addr(sx, sy, ex, ey, wp_cb0_start), 6u * 2048, nd, true);";
    assert_eq!(
        printed_multicast_payloads(&[line.to_string()]),
        BTreeSet::from([6])
    );
    assert!(printed_multicast_payloads(&["noc_async_read_tile(t, s, wp);".to_string()]).is_empty());
}
