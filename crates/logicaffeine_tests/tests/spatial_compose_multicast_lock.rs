//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A COMPOSITION ON A MULTICASTING TARGET: EVERY GROUP COUNTS ITS OWN SECTION'S ITEMS.
//!
//! On a target that multicasts (Tensix), a matmul section places row and column groups whose sender and
//! receivers must run the same number of items — every round the sender sends, every receiver receives. In a
//! composed program a site holds items of EVERY section, so the rule counts the items of the GROUP'S section at
//! each member site; counting the site's items across all sections refuses any composition whose sections place
//! different item counts on the group's sites, which is every composition of two differently shaped matmuls.
//!
//! Pinned end to end on Tensix: two chained integer matmuls compose, certify, and equal the VM exactly.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_compose_multicast_lock`
//!
//! ★ MUTATION PROBE (run when this landed): count `pl.items_at(site)` — the site's items across every section —
//! in `validate_placement`'s group rule, and the chain is refused `ItemCountMismatch { group: 0 }` at compose.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

fn chain() -> String {
    let mut src = String::from("## Main\nLet X be a 32 by 64 tensor of Int.\nLet W be a 64 by 32 tensor of Int.\nLet U be a 32 by 256 tensor of Int.\n");
    for i in 1..=32 * 64 {
        src += &format!("Set item {i} of X to {}.\n", (i * 7) % 5 - 2);
    }
    for i in 1..=64 * 32 {
        src += &format!("Set item {i} of W to {}.\n", (i * 3) % 5 - 2);
    }
    for i in 1..=32 * 256 {
        src += &format!("Set item {i} of U to {}.\n", (i * 11) % 5 - 2);
    }
    src += "Let Z be X matmul W.\nLet V be Z matmul U.\n";
    src
}

#[test]
fn two_multicast_matmul_sections_compose_on_tensix_and_equal_the_vm() {
    let target = SpatialTarget::wormhole();
    let src = chain();
    let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
    let p = l.program.unwrap_or_else(|| {
        panic!(
            "the chain composes and certifies on Tensix: {:?}",
            l.missing
        )
    });
    let groups = p.placement.as_ref().map(|pl| pl.groups.len()).unwrap_or(0);
    assert!(groups > 0, "the precondition: the sections multicast");
    let per_site: Vec<(usize, usize)> = p
        .placement
        .as_ref()
        .map(|pl| {
            pl.items_of_site
                .iter()
                .map(|(s, _)| {
                    (
                        p.items_at_in(Some(0), *s).len(),
                        p.items_at_in(Some(1), *s).len(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        per_site.iter().any(|(a, b)| a != b),
        "the precondition: the two sections place different item counts on a site: {per_site:?}"
    );
    let out =
        eval_choreography(&p, &inputs_from_fills(&p)).expect("the composed program evaluates");
    let v = p.tensors.iter().find(|t| t.name == "V").expect("V").id;
    let got: Vec<i64> = match out.tensors.get(&v).expect("V is written") {
        Value::Int(x) => x.clone(),
        Value::Float(x) => x.iter().map(|f| *f as i64).collect(),
    };
    let mut shown = src.clone();
    for i in [1, 2, 255, 256, 257, 32 * 256] {
        shown += &format!("Show item {i} of V.\n");
    }
    let vm = vm_outcome(&shown);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer"))
        .collect();
    let ours: Vec<i64> = [1usize, 2, 255, 256, 257, 32 * 256]
        .iter()
        .map(|i| got[i - 1])
        .collect();
    assert_eq!(
        ours, want,
        "the composed Tensix program is the VM's, exactly"
    );
}
