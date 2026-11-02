//! ★ COOLING REFUSAL LOCK ★ — cooling is a DECLARED budget the compiler refuses against:
//! the same program under the same stack compiles or refuses purely by cooling class, and
//! "meets its budget under cold-plate cooling, proven" is the certified map's verdict, not
//! the producer's word.
//!
//! THE PROJECTED V1 FINDING this file pins (model, not silicon): the 26.6 mW digital core
//! (sky130_power.json) under two silicon tiers does NOT close at the commodity 85 °C ceiling on
//! airflow alone — natural convection reaches ~364 °C and forced air ~87 °C; a cold plate closes
//! at ~28.5 °C.
//! Analog-over-hot-digital is exactly the configuration the thermal machinery exists to
//! expose. If the model's figures change, these pins move CONSCIOUSLY with the dossier.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. A refusal never becomes a silent pass; the
//! monotonicity of cooling classes is physics.

use logicaffeine_compile::codegen_ana::stack::{CoolingClass, StackTarget};
use logicaffeine_compile::codegen_ana::stack_place::place_stack;
use logicaffeine_compile::codegen_ana::thermal::{
    check_cooling, thermal_default_opts, thermal_map,
};
use logicaffeine_compile::codegen_ana::{analog_netlist_from_source, AnaLowerError, AnalogTarget};

const DOT3: &str = "## To dot3 (a: Float, b: Float, c: Float) -> Float:\n    Assume a is between -1.0 and 1.0.\n    Assume b is between -1.0 and 1.0.\n    Assume c is between -1.0 and 1.0.\n    Return a * 0.25 + b * 0.5 + c * 0.125.\n\n## Main\n    Show dot3(0.5, 0.25, 0.75).\n";

fn peak_under(class: CoolingClass) -> (f64, Result<(), AnaLowerError>) {
    let netlist =
        analog_netlist_from_source(DOT3, "dot3", &AnalogTarget::sky130_fpaa()).expect("lowers");
    let mut stack = StackTarget::sky130_stack_v1();
    stack.cooling.top = class;
    let placement = place_stack(&netlist, &stack, 4.0).expect("places");
    let map = thermal_map(&stack, &netlist, &placement, &thermal_default_opts()).expect("solves");
    (map.delta_t_c[map.hottest], check_cooling(&map, &stack))
}

#[test]
fn the_same_program_compiles_or_refuses_purely_by_cooling_class() {
    let (peak_nat, nat) = peak_under(CoolingClass::NaturalConvection);
    let (peak_air, air) = peak_under(CoolingClass::ForcedAir);
    let (peak_plate, plate) = peak_under(CoolingClass::ColdPlate);

    // Physics is monotone in the cooling class.
    assert!(
        peak_plate < peak_air && peak_air < peak_nat,
        "{peak_plate} < {peak_air} < {peak_nat}"
    );

    // The projected V1 finding: airflow alone does not close this stack at 85 °C.
    match nat {
        Err(AnaLowerError::ThermalBudgetExceeded {
            t_junction_c,
            t_max_c,
            cooling,
            tier,
            ..
        }) => {
            assert!(
                t_junction_c > 300.0,
                "natural convection is catastrophic here: {t_junction_c}"
            );
            assert_eq!(t_max_c, 85.0);
            assert_eq!(cooling, "natural-convection");
            assert_eq!(tier, 0, "the refusal names the digital tier's junction");
        }
        other => panic!("natural convection must refuse, got {other:?}"),
    }
    match air {
        Err(AnaLowerError::ThermalBudgetExceeded { t_junction_c, .. }) => {
            assert!(
                (85.0..150.0).contains(&t_junction_c),
                "forced air misses the ceiling by tens of degrees, not orders: {t_junction_c}"
            );
        }
        other => panic!("forced air must refuse under this model, got {other:?}"),
    }
    plate.expect("the cold plate closes the budget — the SAME program now compiles");

    // The refusal explains itself.
    let msg = peak_under(CoolingClass::NaturalConvection)
        .1
        .unwrap_err()
        .to_string();
    assert!(
        msg.contains("natural-convection") && msg.contains("85"),
        "{msg}"
    );
}
