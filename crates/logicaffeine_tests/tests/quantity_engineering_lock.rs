//! ★ LOCK — quantities carry engineering arithmetic exactly, on every tier. ★
//!
//! The formulas an engineer writes about a solar array, a battery bank or a cable run: a string's
//! open-circuit voltage at its coldest morning, the drop over a wire, a battery's watt-hours, a
//! derate. Each row is one program and its exact answer, and every row must print that answer on the
//! tree-walker and the VM, on the compiled Rust, and on the C backend:
//!
//! - two temperatures on an offset scale subtract to a temperature *difference* (`Δ°C`), which adds
//!   back onto a temperature, scales a temperature coefficient, and converts `in kelvin`; adding two
//!   such temperatures is refused, because it has no physical meaning;
//! - a product or quotient whose dimension is an SI unit with a name shows in that unit
//!   (`W`, `V`, `A`, `Ω`, `J`, …), and one whose dimension cancels is a plain number;
//! - a duration multiplies a quantity (`200 watts * 2 hours` is energy);
//! - a quantity scales by an exact `Rational` or `Decimal`, and refuses a `Float`, which is not exact;
//! - a unit is found by its symbol (case-sensitive: `A` is the ampere, `a` the are) and by every
//!   spelling of its name, including names of two words.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test quantity_engineering_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, assert_exact_output, assert_interpreter_output};

/// One program, one exact answer, on every tier.
fn everywhere(source: &str, expected: &str) {
    assert_interpreter_output(source, expected);
    assert_exact_output(source, expected);
    assert_c_output(source, expected);
}

fn main_of(body: &str) -> String {
    format!("## Main\n{body}\n")
}

// ---- Temperatures on an offset scale ----

#[test]
fn two_celsius_temperatures_subtract_to_a_difference() {
    everywhere(&main_of("Show 25 celsius - 20 celsius."), "5 Δ°C");
}

#[test]
fn a_cold_morning_against_standard_test_conditions() {
    everywhere(&main_of("Show quantity(0 - 10, \"celsius\") - 25 celsius."), "-35 Δ°C");
}

#[test]
fn a_celsius_difference_is_the_same_number_of_kelvin() {
    everywhere(&main_of("Show (25 celsius - 20 celsius) in kelvin."), "5 K");
}

#[test]
fn two_fahrenheit_temperatures_subtract_to_a_fahrenheit_difference() {
    everywhere(&main_of("Show 70 fahrenheit - 50 fahrenheit."), "20 Δ°F");
    everywhere(&main_of("Show (70 fahrenheit - 50 fahrenheit) in kelvin."), "100/9 K");
}

#[test]
fn a_difference_adds_back_onto_a_temperature() {
    everywhere(&main_of("Show 20 celsius + (30 celsius - 25 celsius)."), "25 °C");
    everywhere(&main_of("Show 20 celsius - (30 celsius - 25 celsius)."), "15 °C");
    everywhere(&main_of("Show 20 celsius + 5 kelvin."), "25 °C");
}

#[test]
fn a_difference_scales_a_temperature_coefficient() {
    // 2 V/K over a 5 K rise is 10 V.
    everywhere(
        &main_of("Let rise be 30 celsius - 25 celsius.\nShow 2 volts * rise / quantity(1, \"kelvin\")."),
        "10 V",
    );
}

#[test]
fn the_cold_open_circuit_voltage_of_a_solar_string() {
    // 3 × 22.8 V × (1 + (−0.003/K)(−35 K)) = 75.582 V, exactly.
    everywhere(
        &main_of(
            "Let beta: Rational be (0 - 3) / 1000.\n\
             Let voc be 22.8 volts * 3.\n\
             Let dt be quantity(0 - 10, \"celsius\") - 25 celsius.\n\
             Show voc + voc * dt / quantity(1, \"kelvin\") * beta.",
        ),
        "37791/500 V",
    );
}

#[test]
fn two_offset_temperatures_do_not_add() {
    for source in [main_of("Show 20 celsius + 5 celsius."), main_of("Show 20 fahrenheit + 5 fahrenheit.")] {
        let r = common::run_interpreter(&source);
        assert!(!r.success, "adding two temperatures must be refused:\n{source}");
        assert!(r.error.contains("temperature"), "the refusal names temperatures, got: {}", r.error);
        common::assert_panics(&source, "temperature");
    }
}

// ---- Named SI units for products and quotients ----

#[test]
fn power_is_shown_in_watts() {
    everywhere(&main_of("Show 10.2 amperes * 20.4 volts."), "5202/25 W");
}

#[test]
fn ohms_law_is_shown_in_amperes_and_volts() {
    everywhere(&main_of("Show 24 volts / 2 ohms."), "12 A");
    everywhere(&main_of("Show 3 amperes * 4 ohms."), "12 V");
}

#[test]
fn energy_is_shown_in_joules_and_converts_to_watt_hours() {
    everywhere(&main_of("Show 200 watts * 3600 seconds."), "720000 J");
    everywhere(&main_of("Show convert(200 watts * 3600 seconds, \"watt hours\")."), "200 Wh");
}

#[test]
fn a_battery_bank_in_watt_hours() {
    everywhere(&main_of("Show convert(quantity(100, \"ah\") * 51.2 volts * 2, \"watt hours\")."), "10240 Wh");
}

#[test]
fn a_loop_voltage_drop_is_shown_in_volts() {
    // 2 × 30 ft × 9.8 A × 0.999 Ω/1000 ft
    everywhere(
        &main_of("Show 9.8 amperes * (0.999 ohms * 30 feet * 2 / quantity(1000, \"feet\"))."),
        "146853/250000 V",
    );
}

#[test]
fn a_dimension_without_a_named_unit_still_shows_its_signature() {
    everywhere(&main_of("Show 2 meters * 3 meters."), "6 L^2");
}

#[test]
fn a_quotient_of_like_quantities_is_a_plain_number() {
    everywhere(&main_of("Show (22.8 volts * 3) / 1 volt."), "342/5");
    everywhere(&main_of("Show (22.8 volts * 3) / 1 volt > 60."), "true");
    everywhere(&main_of("Show 1 meter / 1 meter."), "1");
}

// ---- Durations, Rationals and decimals scale quantities ----

#[test]
fn power_over_a_duration_is_energy() {
    everywhere(&main_of("Show 200 watts * 2 hours."), "1440000 J");
    everywhere(&main_of("Show convert(200 watts * 2 hours, \"watt hours\")."), "400 Wh");
    everywhere(&main_of("Show convert(2 hours * 200 watts, \"watt hours\")."), "400 Wh");
}

#[test]
fn a_distance_over_a_duration_is_a_speed() {
    everywhere(&main_of("Show convert(10 kilometers / 2 hours, \"km/h\")."), "5 km/h");
}

#[test]
fn a_rational_scales_a_quantity_through_further_products() {
    everywhere(
        &main_of("Let k: Rational be 1 / 2.\nLet b be 2 volts * k.\nShow b * 3 kelvin / quantity(1, \"kelvin\")."),
        "3 V",
    );
    everywhere(&main_of("Let k: Rational be 3 / 4.\nShow 1200 watts * k."), "900 W");
    everywhere(&main_of("Let k: Rational be 3 / 4.\nShow k * 1200 watts."), "900 W");
    everywhere(&main_of("Let k: Rational be 3 / 4.\nShow 1200 watts / k."), "1600 W");
}

#[test]
fn a_decimal_scales_a_quantity_exactly() {
    everywhere(&main_of("Let k be decimal(\"1.1\").\nShow 2.5 volts * k."), "11/4 V");
    everywhere(&main_of("Let k be decimal(\"1.25\").\nShow k * 32 amperes."), "40 A");
    everywhere(&main_of("Let k be decimal(\"1.6\").\nShow quantity(k, \"volts\")."), "8/5 V");
    everywhere(&main_of("Let k: Rational be 11 / 10.\nShow 2.5 volts * k."), "11/4 V");
}

#[test]
fn a_float_never_meets_a_quantity() {
    // A binary double is not the number it was written as; an exact quantity refuses it on every tier.
    for source in [
        main_of("Let x be sqrt(2.0).\nShow 1 volt * x."),
        main_of("Let x be sqrt(2.0).\nShow quantity(x, \"volts\")."),
    ] {
        let r = common::run_interpreter(&source);
        assert!(!r.success, "a Float must be refused:\n{source}");
        assert!(r.error.contains("is a Float"), "the refusal names the Float, got: {}", r.error);
        common::assert_panics(&source, "is a Float");
        common::assert_c_traps(&source, "is a Float");
    }
}

// ---- Finding a unit ----

#[test]
fn a_unit_is_found_by_its_symbol() {
    everywhere(&main_of("Show quantity(1, \"V\")."), "1 V");
    everywhere(&main_of("Show quantity(3, \"A\")."), "3 A");
    everywhere(&main_of("Show quantity(2, \"Ω\")."), "2 Ω");
    everywhere(&main_of("Show quantity(5, \"a\")."), "5 a");
}

#[test]
fn a_unit_is_found_by_every_spelling_of_its_name() {
    everywhere(&main_of("Show quantity(50, \"amp hours\")."), "50 Ah");
    everywhere(&main_of("Show quantity(50, \"amp hour\")."), "50 Ah");
    everywhere(&main_of("Show quantity(2, \"newton meters\")."), "2 N·m");
    everywhere(&main_of("Show quantity(1, \"kilowatt hour\")."), "1 kWh");
}
