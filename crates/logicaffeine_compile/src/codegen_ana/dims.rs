//! The ADL dimension census — every numeric field of the characterized descriptors
//! ([`AnalogTarget`](super::target::AnalogTarget), [`NonIdealityModel`](super::nonideal::NonIdealityModel))
//! declares its physical dimension, ONCE, here.
//!
//! Dimensionless is a CLAIM, not a default: counts, ratios, and exponents say so explicitly.
//! `analog_dimension_lock` holds full struct literals of both descriptors, so ADDING a field
//! breaks the lock at compile time until it is consciously censused — and pins the naming
//! convention (`*_v` ⇒ voltage, `*_ns` ⇒ time, `*_k_range` ⇒ frequency) so a field cannot
//! lie about its unit in its name.

use logicaffeine_base::dimension::Dimension;

/// Every numeric field of the ADL, with its declared dimension. Names are
/// `Type::field` — exactly one row per field, no field left out (the lock cross-checks
/// against compile-guarded struct literals).
pub fn adl_dimensions() -> Vec<(&'static str, Dimension)> {
    let d = Dimension::DIMENSIONLESS;
    vec![
        // ── AnalogTarget: array geometry and resources (counts) ──
        ("AnalogTarget::rows", d),
        ("AnalogTarget::cols", d),
        ("AnalogTarget::mul_tiles", d),
        ("AnalogTarget::sum_fanin", d),
        ("AnalogTarget::weight_bits", d),
        ("AnalogTarget::dac_bits", d),
        ("AnalogTarget::adc_bits", d),
        ("AnalogTarget::n_dac", d),
        ("AnalogTarget::n_adc", d),
        // ── AnalogTarget: physical figures ──
        ("AnalogTarget::rails_v", Dimension::voltage()),
        ("AnalogTarget::headroom", d),
        ("AnalogTarget::weight_range", d),
        ("AnalogTarget::mul_vref_options_v", Dimension::voltage()),
        ("AnalogTarget::scale_log2_range", d),
        ("AnalogTarget::integrator_k_range", Dimension::frequency()),
        ("AnalogTarget::settle_ns", Dimension::time()),
        // ── NonIdealityModel ──
        ("NonIdealityModel::weight_gain_sigma", d),
        ("NonIdealityModel::gm_distance_sigma_per_pitch", d),
        ("NonIdealityModel::offset_sigma_v", Dimension::voltage()),
        ("NonIdealityModel::noise_floor_v", Dimension::voltage()),
        ("NonIdealityModel::cross_tier_gain_sigma", d),
        (
            "NonIdealityModel::gain_tempco_per_k",
            Dimension::temperature().recip(),
        ),
        (
            "NonIdealityModel::offset_tempco_v_per_k",
            Dimension::voltage().div(Dimension::temperature()),
        ),
        ("NonIdealityModel::chop_residual_v", Dimension::voltage()),
    ]
}
