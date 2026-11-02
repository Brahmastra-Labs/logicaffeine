//! Regenerate `benchmarks/silicon/thermal_projections.json` from the CERTIFIED thermal model
//! — every figure PROJECTED (exact-ℚ `G·ΔT = P` on the modeled V1 stack), none measured; a
//! thermal camera meeting first silicon retires the file's honesty rider, not its schema.
//!
//! Run: `cargo run -p logicaffeine-compile --example gen_thermal_projections`
//! (`cooling_provenance_lock` re-derives every number and pins the file to the model, so a
//! stale JSON goes RED instead of quietly lying.)

use logicaffeine_compile::codegen_ana::stack::{CoolingClass, StackTarget};
use logicaffeine_compile::codegen_ana::stack_place::place_stack;
use logicaffeine_compile::codegen_ana::thermal::{
    check_cooling, cooling_class_name, thermal_default_opts, thermal_map,
};
use logicaffeine_compile::codegen_ana::{analog_netlist_from_source, AnalogTarget};

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

fn main() {
    let netlist =
        analog_netlist_from_source(DOT3, "dot3", &AnalogTarget::sky130_fpaa()).expect("lowers");
    let mut entries = Vec::new();
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
        let t_junction = stack.cooling.ambient_c + peak;
        let meets = check_cooling(&map, &stack).is_ok();
        entries.push(format!(
            "    \"{}\": {{ \"peak_delta_t_c\": {peak}, \"t_junction_c\": {t_junction}, \"meets_85c_budget\": {meets} }}",
            cooling_class_name(class)
        ));
    }
    let provenance = "ALL PROJECTED from the certified thermal model (exact-Q G*dT=P solve \
                      over the modeled V1 stack; geometry and film coefficients are MODELED \
                      design parameters per benchmarks/silicon/analog/thermal-constants.md; digital power \
                      is the OpenLane signoff figure). No thermal camera has met silicon — \
                      nothing here is a measurement.";
    let json = format!(
        "{{\n  \"_provenance\": \"{provenance}\",\n  \"stack\": \"sky130-stack-v1\",\n  \"region\": \"dot3\",\n  \"ambient_c\": 25.0,\n  \"projections\": {{\n{}\n  }}\n}}\n",
        entries.join(",\n")
    );
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/silicon/thermal_projections.json"
    );
    std::fs::write(path, &json).expect("write projections");
    println!("wrote {path}");
}
