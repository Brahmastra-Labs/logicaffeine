//! Ontology module for bridging anaphora and sort compatibility checking.
//!
//! This module provides:
//! - Part-whole relationship lookup for bridging anaphora resolution
//! - Predicate sort requirements for metaphor detection

use crate::lexicon::Sort;

include!(concat!(env!("OUT_DIR"), "/ontology_data.rs"));

/// Find possible whole objects for a given part noun.
/// Returns None if the noun is not a known part of any whole.
pub fn find_bridging_wholes(part_noun: &str) -> Option<&'static [&'static str]> {
    let wholes = get_possible_wholes(&part_noun.to_lowercase());
    if wholes.is_empty() {
        None
    } else {
        Some(wholes)
    }
}

/// Check if a predicate is compatible with a subject's sort.
/// Returns true if compatible or no sort requirement exists.
pub fn check_sort_compatibility(predicate: &str, subject_sort: Sort) -> bool {
    match get_predicate_sort(&predicate.to_lowercase()) {
        Some(required) => subject_sort.is_compatible_with(required),
        None => true,
    }
}

/// Get the required sort for a predicate, if any.
pub fn required_sort(predicate: &str) -> Option<Sort> {
    get_predicate_sort(&predicate.to_lowercase())
}
