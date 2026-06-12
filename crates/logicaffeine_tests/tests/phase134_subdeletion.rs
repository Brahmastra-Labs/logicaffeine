//! Phase 134 — §2.4 Comparative subdeletion (MISSING_ENGLISH.md).
//!
//! A clausal `than`-complement with its OWN gradable dimension compares two
//! different degrees:
//!   "The desk is longer than the door is wide."
//!     → max{d : Long(desk,d)} > max{d' : Wide(door,d')}
//! The two adjectives (long vs wide) measure distinct dimensions.

use logicaffeine_language::compile;

#[test]
fn subdeletion_compares_two_dimensions() {
    let out = compile("The desk is longer than the door is wide.").unwrap();
    eprintln!("subdeletion: {out}");
    // Both gradable dimensions must be present.
    assert!(out.contains("Long"), "the matrix dimension (length): {out}");
    assert!(out.contains("Wide"), "the than-clause dimension (width) must NOT be dropped: {out}");
    assert!(out.contains("Desk"), "the matrix subject: {out}");
    assert!(out.contains("Door"), "the than-clause subject: {out}");
    // A degree comparison between them.
    assert!(out.contains('>') || out.contains("max"), "compares the two degrees: {out}");
}
