//! ★ ANALOG DIMENSION LOCK ★ — the ADL is dimensioned: every numeric descriptor field
//! declares its physical dimension, the physical ones carry SI newtypes, and no pinned value
//! moved in the flip.
//!
//! What this file locks:
//!   1. `logicaffeine_base::si` newtypes agree with the exact `Dimension` algebra — including
//!      the rational-exponent payoff (`VoltsPerRootHz` = V·Hz^(−1/2)).
//!   2. `adl_dimensions()` is a TOTAL, exact census of every numeric field of `AnalogTarget`
//!      and `NonIdealityModel` (full struct literals here make a new field a compile error
//!      until censused), and the naming convention cannot lie: `*_v` ⇒ voltage, `*_ns` ⇒
//!      time, `*_k_range` ⇒ frequency.
//!   3. The physical fields are TYPED (`Volts`, `Nanoseconds`, `Hertz`) and every pinned
//!      preset value is bit-identical to its pre-flip self — the flip moved types, not values.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. Untyping a field or skipping the census is a
//! regression; dimension claims only ever get MORE precise.

use std::collections::BTreeSet;

use logicaffeine_base::dimension::Dimension;
use logicaffeine_base::si::{
    Hertz, Kelvin, Nanoseconds, PerKelvin, Seconds, Siemens, Volts, VoltsPerRootHz, Watts,
};
use logicaffeine_compile::codegen_ana::dims::adl_dimensions;
use logicaffeine_compile::codegen_ana::{sky130_nonideal, AnalogTarget, NonIdealityModel};

#[test]
fn si_newtypes_carry_the_exact_dimension_algebra() {
    assert_eq!(Volts::dimension(), Dimension::voltage());
    assert_eq!(Seconds::dimension(), Dimension::time());
    assert_eq!(
        Nanoseconds::dimension(),
        Dimension::time(),
        "the unit rides the NAME; the dimension is time"
    );
    assert_eq!(Hertz::dimension(), Dimension::time().recip());
    assert_eq!(Siemens::dimension(), Dimension::conductance());
    assert_eq!(Kelvin::dimension(), Dimension::temperature());
    assert_eq!(
        Watts::dimension(),
        Dimension::energy().div(Dimension::time())
    );
    assert_eq!(PerKelvin::dimension(), Dimension::temperature().recip());

    // The rational-exponent payoff: V·Hz^(−1/2) round-trips through the algebra exactly.
    let vprh = VoltsPerRootHz::dimension();
    assert_eq!(
        vprh,
        Dimension::voltage().div(Dimension::frequency().nth_root(2))
    );
    assert_eq!(
        vprh.mul(Dimension::frequency().nth_root(2)),
        Dimension::voltage()
    );
    assert_eq!(
        vprh.powi(2),
        Dimension::voltage().powi(2).div(Dimension::frequency())
    );
}

#[test]
fn the_adl_census_is_total_exact_and_the_naming_convention_cannot_lie() {
    // FULL struct literals: adding a field to either descriptor breaks THIS at compile time
    // until the census below is consciously extended.
    let _target_census_guard = AnalogTarget {
        name: "guard",
        rows: 0,
        cols: 0,
        mul_tiles: 0,
        rails_v: (Volts(0.0), Volts(0.0)),
        headroom: 0.0,
        cells: BTreeSet::new(),
        sum_fanin: 0,
        weight_bits: 1,
        weight_range: (0.0, 0.0),
        mul_vref_options_v: Vec::<Volts>::new(),
        scale_log2_range: (0, 0),
        dac_bits: 1,
        adc_bits: 1,
        n_dac: 0,
        n_adc: 0,
        integrator_k_range: (Hertz(0.0), Hertz(0.0)),
        settle_ns: Nanoseconds(0.0),
        provenance: "",
    };
    let _model_census_guard = NonIdealityModel {
        weight_gain_sigma: 0.0,
        gm_distance_sigma_per_pitch: 0.0,
        offset_sigma_v: Volts(0.0),
        noise_floor_v: Volts(0.0),
        cross_tier_gain_sigma: 0.0,
        gain_tempco_per_k: 0.0,
        offset_tempco_v_per_k: 0.0,
        chop_residual_v: Volts(0.0),
        provenance: "",
    };

    let census = adl_dimensions();
    let names: BTreeSet<&str> = census.iter().map(|(n, _)| *n).collect();
    assert_eq!(names.len(), census.len(), "duplicate field in the census");
    assert_eq!(
        census.len(),
        24,
        "the ADL field census moved — a conscious edit updates this pin"
    );

    let expected: &[&str] = &[
        "AnalogTarget::rows",
        "AnalogTarget::cols",
        "AnalogTarget::mul_tiles",
        "AnalogTarget::sum_fanin",
        "AnalogTarget::weight_bits",
        "AnalogTarget::dac_bits",
        "AnalogTarget::adc_bits",
        "AnalogTarget::n_dac",
        "AnalogTarget::n_adc",
        "AnalogTarget::rails_v",
        "AnalogTarget::headroom",
        "AnalogTarget::weight_range",
        "AnalogTarget::mul_vref_options_v",
        "AnalogTarget::scale_log2_range",
        "AnalogTarget::integrator_k_range",
        "AnalogTarget::settle_ns",
        "NonIdealityModel::weight_gain_sigma",
        "NonIdealityModel::gm_distance_sigma_per_pitch",
        "NonIdealityModel::offset_sigma_v",
        "NonIdealityModel::noise_floor_v",
        "NonIdealityModel::cross_tier_gain_sigma",
        "NonIdealityModel::gain_tempco_per_k",
        "NonIdealityModel::offset_tempco_v_per_k",
        "NonIdealityModel::chop_residual_v",
    ];
    let expected_set: BTreeSet<&str> = expected.iter().copied().collect();
    assert_eq!(
        names, expected_set,
        "census ≡ the descriptors' numeric fields, exactly"
    );

    for (name, dim) in &census {
        if name.ends_with("_v") || name.ends_with("_options_v") {
            assert_eq!(
                *dim,
                Dimension::voltage(),
                "`{name}` is named a voltage — it must BE one"
            );
        }
        if name.ends_with("_ns") {
            assert_eq!(*dim, Dimension::time(), "`{name}` is named a time");
        }
        if name.ends_with("_k_range") {
            assert_eq!(*dim, Dimension::frequency(), "`{name}` is named a rate");
        }
    }
    // Spot coherence with the si types.
    let get = |n: &str| census.iter().find(|(m, _)| *m == n).unwrap().1;
    assert_eq!(get("AnalogTarget::rails_v"), Volts::dimension());
    assert_eq!(get("AnalogTarget::settle_ns"), Nanoseconds::dimension());
    assert_eq!(get("AnalogTarget::integrator_k_range"), Hertz::dimension());
    assert_eq!(get("AnalogTarget::rows"), Dimension::DIMENSIONLESS);
    assert_eq!(
        get("NonIdealityModel::weight_gain_sigma"),
        Dimension::DIMENSIONLESS
    );
}

#[test]
fn the_descriptor_fields_are_typed_and_no_pinned_value_moved_in_the_flip() {
    let t = AnalogTarget::ideal();
    let (lo, hi): (Volts, Volts) = t.rails_v;
    assert_eq!((lo.0, hi.0), (0.0, 1.8));
    let s: Nanoseconds = t.settle_ns;
    assert_eq!(s.0, 100.0);
    let (klo, khi): (Hertz, Hertz) = t.integrator_k_range;
    assert_eq!((klo.0, khi.0), (1.0, 1.0e9));
    let v0: Volts = t.mul_vref_options_v[0];
    assert_eq!(v0.0, 0.5);

    let f = AnalogTarget::sky130_fpaa();
    assert_eq!(
        f.rails_v.1 .0, 1.8,
        "the ONE sourced figure: SKY130 nominal 1.8 V"
    );
    assert_eq!(f.settle_ns.0, 1000.0);
    assert_eq!(
        (f.integrator_k_range.0 .0, f.integrator_k_range.1 .0),
        (1.0e3, 1.0e7)
    );
    assert_eq!(f.mul_vref_options_v[2].0, 0.125);

    let m = sky130_nonideal();
    let o: Volts = m.offset_sigma_v;
    assert_eq!(o.0, 2.8e-3);
    let n: Volts = m.noise_floor_v;
    assert_eq!(n.0, 64.4e-6);
}
