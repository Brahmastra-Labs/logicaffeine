//! ★ LOCK — a quantity is written the way an engineer writes it. ★
//!
//! A number followed by a unit is a quantity, and `… in <unit>` re-expresses one, for every way the unit
//! catalog spells a unit:
//!
//! - a name of several words (`50 amp hours`, `1.6 newton meters`, `… in watt hours`, `25 degrees Celsius`);
//! - a symbol, exactly as SI writes it (`50 Ah`, `5 V`, `3 A`, `9 mA`, `2 Ω`, `2 kWh`, `25 °C`);
//! - a negative magnitude (`-10 celsius`, `-1.5 volts`).
//!
//! Every row prints its answer on the tree-walker and the VM, the compiled Rust, and the C backend.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test quantity_literal_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, assert_exact_output, assert_interpreter_output};

fn everywhere(source: &str, expected: &str) {
    assert_interpreter_output(source, expected);
    assert_exact_output(source, expected);
    assert_c_output(source, expected);
}

fn main_of(body: &str) -> String {
    format!("## Main\n{body}\n")
}

#[test]
fn a_unit_named_in_several_words() {
    everywhere(&main_of("Show 50 amp hours."), "50 Ah");
    everywhere(&main_of("Show 1.6 newton meters."), "8/5 N·m");
    everywhere(&main_of("Show 2 kilowatt hours."), "2 kWh");
}

#[test]
fn a_conversion_to_a_unit_named_in_several_words() {
    everywhere(&main_of("Show 1 kWh in watt hours."), "1000 Wh");
    everywhere(&main_of("Show 100 amp hours in milliamp hours."), "100000 mAh");
}

#[test]
fn a_unit_written_as_its_symbol() {
    everywhere(&main_of("Show 50 Ah."), "50 Ah");
    everywhere(&main_of("Show 5 V."), "5 V");
    everywhere(&main_of("Show 3 A."), "3 A");
    everywhere(&main_of("Show 9 mA."), "9 mA");
    everywhere(&main_of("Show 2 Ω."), "2 Ω");
    everywhere(&main_of("Show 2 kWh."), "2 kWh");
}

#[test]
fn a_temperature_written_with_the_degree_sign() {
    everywhere(&main_of("Show 25 °C."), "25 °C");
    everywhere(&main_of("Show 77 °F in celsius."), "25 °C");
    everywhere(&main_of("Show 30 °C - 25 °C."), "5 Δ°C");
}

#[test]
fn a_temperature_written_in_degrees() {
    everywhere(&main_of("Show 25 degrees Celsius."), "25 °C");
    everywhere(&main_of("Show 77 degrees fahrenheit in degrees celsius."), "25 °C");
}

#[test]
fn a_negative_quantity() {
    everywhere(&main_of("Show -10 celsius."), "-10 °C");
    everywhere(&main_of("Show -1.5 volts."), "-3/2 V");
    everywhere(&main_of("Show 2 volts - -1 volt."), "3 V");
}

#[test]
fn a_force_in_newtons() {
    everywhere(&main_of("Show 3 newtons."), "3 N");
    everywhere(
        &main_of("Show (2 kilograms * 3 meters / 1 second / 1 second) in newtons."),
        "6 N",
    );
}

#[test]
fn a_number_before_an_ordinary_word_is_still_a_number() {
    everywhere(&main_of("Let a be 3.\nShow 2 * a."), "6");
    everywhere(&main_of("Let n be 4.\nShow n."), "4");
}
