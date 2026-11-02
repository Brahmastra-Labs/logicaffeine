//! ★ ANALOG ARTIFACT LOCK ★ — `compile_to_analog` produces a COMPLETE, self-consistent bundle:
//! the lowered region, the physical schedule and placement, both certificates (range + budget)
//! that replay from their own contents, and a SPICE netlist that pins its simulator options.
//! Structural honesty: the netlist contains ONLY behavioral elements (no invented device model
//! cards), one source per plane input, one controlled source per tile.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::to_range_claim;
use logicaffeine_compile::compile::compile_to_analog;
use logicaffeine_proof::interval_replay::check_range_claim;

const PROGRAM: &str = "## To blend (a: Float, b: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Return a * 0.5 + b * 0.25.\n\n## Main\n    Show blend(0.5, 0.5).\n";

#[test]
fn the_artifact_is_complete_and_self_consistent() {
    let art = compile_to_analog(PROGRAM).expect("the pipeline compiles the region");
    assert_eq!(art.fn_name, "blend");
    assert_eq!(art.netlist.n_in, 2);
    assert!(!art.netlist.cells.is_empty());
    assert_eq!(art.placement.site.len(), art.schedule.quantized.cells.len());

    // Both certificates replay from their own contents.
    assert!(art.budget.recheck(), "budget must replay");
    let (cells, claim) = to_range_claim(&art.schedule, &art.cert);
    assert!(
        check_range_claim(&cells, &claim),
        "the range claim must pass the independent checker"
    );

    // The SPICE bundle is structurally honest.
    assert!(
        art.spice.contains(".options reltol="),
        "simulator options pinned in the netlist"
    );
    let sources = art.spice.lines().filter(|l| l.starts_with("VIN")).count();
    assert_eq!(sources, art.netlist.n_in, "one DC source per plane input");
    let tiles = art
        .spice
        .lines()
        .filter(|l| l.starts_with("BSUM") || l.starts_with("BMUL") || l.starts_with("BGAIN"))
        .count();
    assert_eq!(
        tiles,
        art.schedule.quantized.cells.len(),
        "one behavioral element per tile"
    );
    for line in art.spice.lines() {
        assert!(
            !line.starts_with(".model") && !line.starts_with(".MODEL"),
            "no invented device model cards in the behavioral tier: {line}"
        );
    }
}

#[test]
fn a_contractless_program_still_refuses_at_the_pipeline_surface() {
    let no_ranges = PROGRAM.replace("    Assume a is between -1.0 and 1.0.\n", "");
    let err = compile_to_analog(&no_ranges).expect_err("a missing contract refuses");
    let msg = format!("{err:?}");
    assert!(msg.contains("analog backend:"), "{msg}");
    assert!(msg.contains("declared range"), "{msg}");
}
