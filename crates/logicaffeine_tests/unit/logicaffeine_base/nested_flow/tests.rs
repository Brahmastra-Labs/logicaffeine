//! `flow` and `power` take a start state with one value per row of the update. A recipe read off
//! the wire (tags 12 and 13 of `NestedGerm::read_recipe`) carries that state as a vector of any
//! length, so both refuse a state of the wrong length by name, and a forged encoding decodes to
//! `None` instead of indexing past the state or cutting it short.

use super::*;
use crate::limit_value::ResidueType;

fn scale() -> Arc<Scale> {
    Arc::new(Scale::new("dt", ResidueType::any()))
}

fn int(x: i64) -> Number {
    rational(Rational::from_i64(x))
}

#[test]
fn the_scalar_power_refuses_an_empty_start_state_instead_of_panicking() {
    let refused = power(&scale(), &vec![vec![int(2)]], int(3), &[]);
    assert!(refused.is_err(), "power of [[2]] from an empty state was {refused:?}");
}

#[test]
fn the_scalar_power_refuses_a_start_state_longer_than_the_update() {
    let refused = power(&scale(), &vec![vec![int(2)]], int(3), &[int(1), int(1)]);
    assert!(refused.is_err(), "power of [[2]] from a two-value state was {refused:?}");
}

#[test]
fn a_matrix_power_refuses_a_short_start_state() {
    let identity2 = vec![vec![int(1), int(0)], vec![int(0), int(1)]];
    let refused = power(&scale(), &identity2, int(3), &[int(1)]);
    assert!(refused.is_err(), "power of a 2x2 update from a one-value state was {refused:?}");
}

#[test]
fn the_flow_refuses_a_start_state_of_the_wrong_length_instead_of_truncating_it() {
    let identity2 = vec![vec![int(1), int(0)], vec![int(0), int(1)]];
    for start in [vec![], vec![int(1)], vec![int(1), int(2), int(3)]] {
        let refused = flow(&scale(), &identity2, Rational::from_i64(1), Rational::from_i64(0), &start);
        assert!(refused.is_err(), "flow of a 2x2 update from {} values was {refused:?}", start.len());
    }
}

#[test]
fn the_refusal_names_both_lengths() {
    let message = power(&scale(), &vec![vec![int(2)]], int(3), &[]).unwrap_err();
    assert!(message.contains('1') && message.contains('0'), "the refusal does not say which lengths differ: {message}");
}
