//! ★ COOLING PROVENANCE LOCK ★ — `benchmarks/silicon/thermal_projections.json` is pinned to
//! the model that generated it: every number re-derives, and the file SAYS it is projected.
//!
//! A stale or hand-edited projection goes RED here instead of quietly shipping; the honesty
//! rider survives until a thermal camera meets silicon.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE.

use logicaffeine_compile::codegen_ana::stack::{CoolingClass, StackTarget};
use logicaffeine_compile::codegen_ana::stack_place::place_stack;
use logicaffeine_compile::codegen_ana::thermal::{
    check_cooling, cooling_class_name, thermal_default_opts, thermal_map,
};
use logicaffeine_compile::codegen_ana::{analog_netlist_from_source, AnalogTarget};

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

const JSON: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../benchmarks/silicon/thermal_projections.json"
);

#[test]
fn the_projections_file_re_derives_from_the_model_and_says_projected() {
    let text = std::fs::read_to_string(JSON).expect("thermal_projections.json exists");
    assert!(
        text.contains("ALL PROJECTED"),
        "the honesty rider leads the file"
    );
    assert!(text.contains("nothing here is a measurement"));
    assert!(
        text.contains("benchmarks/silicon/analog/thermal-constants.md"),
        "the dossier is named"
    );

    let netlist =
        analog_netlist_from_source(DOT3, "dot3", &AnalogTarget::sky130_fpaa()).expect("lowers");
    for class in [
        CoolingClass::NaturalConvection,
        CoolingClass::ForcedAir,
        CoolingClass::ColdPlate,
    ] {
        let mut stack = StackTarget::sky130_stack_v1();
        stack.cooling.top = class;
        let placement = place_stack(&netlist, &stack, 4.0).expect("places");
        let map =
            thermal_map(&stack, &netlist, &placement, &thermal_default_opts()).expect("solves");
        let peak = map.delta_t_c[map.hottest];
        let meets = check_cooling(&map, &stack).is_ok();
        let expected = format!(
            "\"{}\": {{ \"peak_delta_t_c\": {peak}, \"t_junction_c\": {}, \"meets_85c_budget\": {meets} }}",
            cooling_class_name(class),
            stack.cooling.ambient_c + peak
        );
        assert!(
            text.contains(&expected),
            "stale projection for {}: regenerate with `cargo run -p logicaffeine-compile \
             --example gen_thermal_projections`\n  wanted: {expected}",
            cooling_class_name(class)
        );
    }
}
